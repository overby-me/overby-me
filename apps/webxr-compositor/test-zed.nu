#!/usr/bin/env nu

# test-zed.nu: the end goal, Zed editing in the browser.
#
# Runs the user's Zed (stateless, isolated XDG dirs) on the compositor with
# software Vulkan (lavapipe presents through wl_shm), accepts the trust
# dialog, clicks into the buffer and types; the typed glyphs must appear as
# new bright pixels on the canvas.
#
# Needs `zeditor` on PATH (the nixpkgs zed-editor wrapper).
#
# Exit codes: 0 Zed renders and accepts typing · 1 it does not · 2 setup
# failed.

const HOST_PORT = 8387
const CDP_PORT = 9238
const SOCKET = "wayland-webxr-zed"
const BUNDLE = "target/dx/webxr-compositor/release/web/public"
const HOST_BIN = "host/target/debug/webxr-compositor"

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset) ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset) ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset) ($msg | str join ' ')" }

def find-chromium [] {
    let found = (
        ls /nix/store
        | where type == dir
        | get name
        | where {|d| ($d | path basename) =~ '-chromium-[0-9]' }
        | each {|d| $"($d)/bin/chromium" }
        | where {|p| ($p | path exists) }
        | sort
    )
    if ($found | is-empty) { null } else { $found | last }
}

const DRIVER = '
const [cdp, url, out] = Deno.args;

const targets = await (await fetch(`http://127.0.0.1:${cdp}/json`)).json();
const page = targets.find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((ok) => ws.onopen = ok);

let next = 1;
const pending = new Map();
const lines = [];
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg.result);
    pending.delete(msg.id);
  } else if (msg.method === "Runtime.exceptionThrown") {
    const d = msg.params.exceptionDetails;
    const desc = d.exception?.description ?? d.exception?.value ?? "";
    lines.push("EXCEPTION " + d.text + (desc ? ": " + String(desc).split("\n")[0] : ""));
  }
};
const send = (method, params = {}) =>
  new Promise((ok) => {
    const id = next++;
    pending.set(id, ok);
    ws.send(JSON.stringify({ id, method, params }));
  });

await send("Page.enable");
await send("Runtime.enable");
const pause = (ms) => new Promise((ok) => setTimeout(ok, ms));
const read = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, returnByValue: true });
  return r.result?.value;
};

await send("Page.navigate", { url });

// Zed takes a while to boot; wait for a wide canvas with pixels.
const geometry = `(() => {
  const c = document.querySelector("canvas.surface");
  if (!c || c.width < 800) return null;
  const r = c.getBoundingClientRect();
  return { w: c.width, h: c.height, left: r.left, top: r.top };
})()`;
let geo = null;
for (let i = 0; i < 360; i++) {
  geo = await read(geometry);
  if (geo) break;
  await pause(250);
}
const result = { steps: {}, console: lines };
if (!geo) {
  console.log(JSON.stringify(result));
  ws.close();
  Deno.exit(0);
}
result.steps.mounted = { w: geo.w, h: geo.h };
await pause(3000);

// Bright pixels in the visible editor band (glyphs on the dark theme).
const snapshot = `(() => {
  const c = document.querySelector("canvas.surface");
  const h = Math.min(c.height, 500);
  const d = c.getContext("2d").getImageData(0, 150, Math.min(c.width, 1000), h - 150).data;
  let lit = 0;
  for (let i = 0; i < d.length; i += 4) {
    if (d[i] + d[i + 1] + d[i + 2] > 350) lit++;
  }
  return lit;
})()`;

const click = async (x, y) => {
  await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
};
const tap = async (code, key) => {
  await send("Input.dispatchKeyEvent", { type: "keyDown", code, key });
  await send("Input.dispatchKeyEvent", { type: "keyUp", code, key });
  await pause(60);
};

// Focus, accept the trust dialog (Enter), and land in the buffer.
await click(geo.left + 200, geo.top + 215);
await pause(500);
await tap("Enter", "Enter");
await pause(1500);
await click(geo.left + 280, geo.top + 215);
await pause(500);

const before = await read(snapshot);

await tap("End", "End");
await tap("Enter", "Enter");
const keys = [
  ["KeyZ", "z"], ["KeyE", "e"], ["KeyD", "d"], ["Space", " "],
  ["KeyI", "i"], ["KeyN", "n"], ["Space", " "],
  ["KeyB", "b"], ["KeyR", "r"], ["KeyO", "o"], ["KeyW", "w"],
  ["KeyS", "s"], ["KeyE", "e"], ["KeyR", "r"],
];
for (const [code, key] of keys) await tap(code, key);
await pause(2000);

const after = await read(snapshot);
result.steps.typed = { before, after };

const shot = await send("Page.captureScreenshot", { format: "png" });
await Deno.writeFile(out, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));

for (const line of lines) console.error("  " + line);
console.log(JSON.stringify(result));
ws.close();
'

def chromium-args [] {
    [
        "--headless=new"
        "--no-sandbox"
        "--disable-dev-shm-usage"
        "--hide-scrollbars"
        "--window-size=1280,800"
        $"--remote-debugging-port=($CDP_PORT)"
        "--remote-allow-origins=*"
        "about:blank"
    ]
}

def main [--out: string = "/tmp/webxr-compositor-zed.png"]: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    if not ($root | path exists) {
        log-fail $"no bundle at ($root); run `just build` first"
        exit 2
    }
    if not ($host_bin | path exists) {
        log-fail $"no host at ($host_bin); run `cargo build --manifest-path host/Cargo.toml` first"
        exit 2
    }
    if (which zeditor | is-empty) {
        log-fail "no zeditor on PATH; install zed-editor"
        exit 2
    }
    let mesa = (^nix build --no-link --print-out-paths "nixpkgs#mesa" | complete)
    if $mesa.exit_code != 0 {
        log-fail "could not resolve nixpkgs#mesa for the lavapipe ICD"
        exit 2
    }
    let icd = $"($mesa.stdout | str trim)/share/vulkan/icd.d/lvp_icd.x86_64.json"
    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }

    # Reap leftovers of an aborted earlier run; a survivor keeps the port and
    # the fresh host dies at bind.
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f .zeditor-wrapped | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    let zed_home = (mktemp -d --suffix .webxr-zed)
    mkdir $"($zed_home)/runtime" $"($zed_home)/config" $"($zed_home)/data" $"($zed_home)/cache"
    chmod 700 $"($zed_home)/runtime"
    "hello from webxr\n" | save -f $"($zed_home)/zed-test.txt"

    log-info $"host on ($HOST_PORT), zed on ($SOCKET) via lavapipe, chromium on ($CDP_PORT)"

    let host = (job spawn {
        with-env {
            WEBXR_COMPOSITOR_LISTEN: $"127.0.0.1:($HOST_PORT)"
            WEBXR_COMPOSITOR_WEB_ROOT: $root
            WEBXR_COMPOSITOR_WAYLAND_DISPLAY: $SOCKET
        } { ^$host_bin | complete | ignore }
    })

    mut socket_up = false
    for _ in 0..40 {
        if ($socket_path | path exists) { $socket_up = true; break }
        sleep 250ms
    }
    if not $socket_up {
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        log-fail "the wayland socket never appeared"
        exit 2
    }

    let zed = (job spawn {
        with-env {
            WAYLAND_DISPLAY: $socket_path
            VK_DRIVER_FILES: $icd
            VK_ICD_FILENAMES: $icd
            ZED_ALLOW_EMULATED_GPU: "1"
            ZED_STATELESS: "1"
            XDG_RUNTIME_DIR: $"($zed_home)/runtime"
            XDG_CONFIG_HOME: $"($zed_home)/config"
            XDG_DATA_HOME: $"($zed_home)/data"
            XDG_CACHE_HOME: $"($zed_home)/cache"
        } { ^zeditor --foreground $"($zed_home)/zed-test.txt" | complete | ignore }
    })

    let browser = (job spawn { ^$chromium ...(chromium-args) | complete | ignore })
    mut cdp_up = false
    for _ in 0..60 {
        let ready = (try {
            http get --max-time 1sec $"http://127.0.0.1:($CDP_PORT)/json/version" | is-not-empty
        } catch { false })
        if $ready { $cdp_up = true; break }
        sleep 250ms
    }
    if not $cdp_up {
        try { job kill $browser }
        try { job kill $zed }
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f .zeditor-wrapped | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $out
        | complete
    )
    try { job kill $browser }
    try { job kill $zed }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f .zeditor-wrapped | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
    rm -rf $zed_home

    if $run.exit_code != 0 {
        log-fail $"the driver failed:\n($run.stderr)"
        exit 2
    }
    let report = (try { $run.stdout | from json } catch { null })
    if $report == null {
        log-fail $"the driver said nothing useful:\n($run.stdout)\n($run.stderr)"
        exit 2
    }

    let steps = ($report.steps? | default {})
    mut failures = []
    if ($steps.mounted? | default null) == null {
        $failures = ($failures | append "the Zed window never painted")
    } else {
        let typed = ($steps.typed? | default {before: 0, after: 0})
        let delta = (($typed.after | default 0) - ($typed.before | default 0))
        if $delta < 40 {
            $failures = ($failures | append $"typing lit only ($delta) extra pixels; keystrokes did not reach the editor")
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  steps     ($steps | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "Zed renders and accepts typing in the browser"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
