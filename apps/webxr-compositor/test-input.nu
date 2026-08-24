#!/usr/bin/env nu

# test-input.nu: prove input travels browser → host seat → wayland client.
#
# Starts the host, a real terminal (foot running /bin/sh) on its Wayland
# socket, and headless chromium on its page. The driver clicks the terminal
# window (keyboard focus) and types through CDP, which exercises the page's
# real listeners; the terminal echoing the keystrokes changes its pixels,
# which is what gets asserted.
#
# Exit codes: 0 keystrokes echoed · 1 they did not · 2 setup failed.

const HOST_PORT = 8375
const CDP_PORT = 9226
const SOCKET = "wayland-webxr-input"
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
  } else if (msg.method === "Runtime.consoleAPICalled") {
    lines.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
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

// Wait for the terminal window canvas to carry pixels.
const geometry = `(() => {
  const c = document.querySelector("canvas.surface");
  if (!c || c.width === 0) return null;
  const r = c.getBoundingClientRect();
  return { w: c.width, h: c.height, left: r.left, top: r.top };
})()`;
let geo = null;
for (let i = 0; i < 240; i++) {
  geo = await read(geometry);
  if (geo) break;
  await pause(250);
}
if (!geo) {
  console.log(JSON.stringify({ mounted: false, console: lines }));
  ws.close();
  Deno.exit(0);
}
await pause(1000);

// Bright pixels are glyphs and the cursor: the foot background sums to
// about 110, its foreground text to over 500, so 350 separates them.
const snapshot = `(() => {
  const c = document.querySelector("canvas.surface");
  const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
  let sum = 0, lit = 0;
  for (let i = 0; i < d.length; i += 4) {
    const v = d[i] + d[i + 1] + d[i + 2];
    sum += v;
    if (v > 350) lit++;
  }
  return { sum, lit };
})()`;

const before = await read(snapshot);

// Move then click inside the terminal: pointer focus, then keyboard focus.
const cx = geo.left + Math.min(60, geo.w / 2);
const cy = geo.top + Math.min(40, geo.h / 2);
await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: cx, y: cy });
await send("Input.dispatchMouseEvent", { type: "mousePressed", x: cx, y: cy, button: "left", clickCount: 1 });
await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: cx, y: cy, button: "left", clickCount: 1 });
await pause(300);

const focused = await read(`document.querySelector(".window.focused") !== null`);

const keys = [
  ["KeyE", "e"], ["KeyC", "c"], ["KeyH", "h"], ["KeyO", "o"], ["Space", " "],
  ["KeyW", "w"], ["KeyE", "e"], ["KeyB", "b"], ["KeyX", "x"], ["KeyR", "r"],
];
for (const [code, key] of keys) {
  await send("Input.dispatchKeyEvent", { type: "keyDown", code, key, windowsVirtualKeyCode: 0 });
  await send("Input.dispatchKeyEvent", { type: "keyUp", code, key, windowsVirtualKeyCode: 0 });
  await pause(40);
}
await pause(1200);

const after = await read(snapshot);

const shot = await send("Page.captureScreenshot", { format: "png" });
await Deno.writeFile(out, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));

for (const line of lines) console.error("  " + line);
console.log(JSON.stringify({ mounted: true, geo, focused, before, after, console: lines }));
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

def main [--out: string = "/tmp/webxr-compositor-input.png"]: nothing -> nothing {
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
    if (which foot | is-empty) {
        log-fail "no foot on PATH; enter the devshell"
        exit 2
    }
    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }

    # Reap leftovers of an aborted earlier run; a survivor keeps the port and
    # the fresh host dies at bind.
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
    ^pkill -x foot | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    log-info $"host on ($HOST_PORT), foot on ($SOCKET), chromium on ($CDP_PORT)"

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

    let terminal = (job spawn {
        with-env { WAYLAND_DISPLAY: $SOCKET } { ^foot /bin/sh -i | complete | ignore }
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
        try { job kill $terminal }
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $out
        | complete
    )
    try { job kill $browser }
    try { job kill $terminal }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
    ^pkill -x foot | complete | ignore

    if $run.exit_code != 0 {
        log-fail $"the driver failed:\n($run.stderr)"
        exit 2
    }
    let report = (try { $run.stdout | from json } catch { null })
    if $report == null {
        log-fail $"the driver said nothing useful:\n($run.stdout)\n($run.stderr)"
        exit 2
    }

    mut failures = []
    if not ($report.mounted? | default false) {
        $failures = ($failures | append "the terminal window never appeared on the page")
    } else {
        if not ($report.focused? | default false) {
            $failures = ($failures | append "clicking the window never focused it")
        }
        let before = ($report.before?.lit? | default 0)
        let after = ($report.after?.lit? | default 0)
        if ($after - $before) < 50 {
            $failures = ($failures | append $"typing lit only ($after - $before) extra pixels; keystrokes did not reach the terminal")
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  terminal  ($report.geo? | default {} | to json --raw)"
    print $"  lit px    before ($report.before?.lit? | default 0), after ($report.after?.lit? | default 0)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "keystrokes travelled browser to terminal and echoed"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
