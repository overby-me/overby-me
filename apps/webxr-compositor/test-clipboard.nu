#!/usr/bin/env nu

# test-clipboard.nu: clipboard across windows plus the cursor shape.
#
# Two foot terminals share the compositor. A word typed into the first is
# selected by double click and copied with Ctrl+Shift+C: the host reads the
# client selection through a pipe, mirrors it to the page (asserted on
# window.__wxr.clip; navigator.clipboard.readText is denied in this headless
# chromium so the OS mirror cannot be asserted directly) and takes ownership
# of the wayland selection. Ctrl+Shift+V in the second terminal then pastes
# from the host-owned selection, asserted as new bright pixels. The pointer
# over a terminal must also flip the CSS cursor to "text" via
# cursor-shape-v1.
#
# Exit codes: 0 clipboard flows and cursor follows · 1 they do not ·
# 2 setup failed.

const HOST_PORT = 8379
const CDP_PORT = 9230
const SOCKET = "wayland-webxr-clip"
const BUNDLE = "target/dx/webxr-compositor/release/web/public"
const HOST_BIN = "host/target/debug/webxr-compositor"
const WORD = "marker99"

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
const [cdp, url, word, out] = Deno.args;

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
  const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  return r.result?.value;
};

await send("Page.navigate", { url });
await send("Page.bringToFront");

// Both terminals, ordered by window id (spawn order).
const SURVEY = `(() => {
  const wins = Array.from(document.querySelectorAll(".window")).map((w) => {
    const c = w.querySelector("canvas.surface");
    const r = c.getBoundingClientRect();
    const t = w.querySelector(".titlebar").getBoundingClientRect();
    return {
      id: c.id, w: c.width, h: c.height, left: r.left, top: r.top,
      tx: t.left + t.width / 2, ty: t.top + t.height / 2,
    };
  }).filter((x) => x.w > 0);
  wins.sort((a, b) => a.id.localeCompare(b.id));
  return wins.length === 2 ? wins : null;
})()`;
let wins = null;
for (let i = 0; i < 240; i++) {
  wins = await read(SURVEY);
  if (wins) break;
  await pause(250);
}
const result = { steps: {}, console: lines };
if (!wins) {
  console.log(JSON.stringify(result));
  ws.close();
  Deno.exit(0);
}
result.steps.mounted = true;
await pause(800);

const click = async (x, y, count = 1) => {
  await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: count });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: count });
};

// The cascade overlaps the two windows; drag the second one clear so clicks
// land where they aim.
await send("Input.dispatchMouseEvent", { type: "mousePressed", x: wins[1].tx, y: wins[1].ty, button: "left", clickCount: 1 });
for (let i = 1; i <= 6; i++) {
  await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: wins[1].tx + (480 * i) / 6, y: wins[1].ty });
  await pause(30);
}
await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: wins[1].tx + 480, y: wins[1].ty, button: "left", clickCount: 1 });
await pause(300);
wins = await read(SURVEY);
const [a, b] = wins;
const tap = async (code, key) => {
  await send("Input.dispatchKeyEvent", { type: "keyDown", code, key });
  await send("Input.dispatchKeyEvent", { type: "keyUp", code, key });
  await pause(40);
};
const chord = async (codeKey) => {
  await send("Input.dispatchKeyEvent", { type: "keyDown", code: "ControlLeft", key: "Control" });
  await send("Input.dispatchKeyEvent", { type: "keyDown", code: "ShiftLeft", key: "Shift" });
  await send("Input.dispatchKeyEvent", { type: "keyDown", code: codeKey, key: codeKey.slice(-1) });
  await send("Input.dispatchKeyEvent", { type: "keyUp", code: codeKey, key: codeKey.slice(-1) });
  await send("Input.dispatchKeyEvent", { type: "keyUp", code: "ShiftLeft", key: "Shift" });
  await send("Input.dispatchKeyEvent", { type: "keyUp", code: "ControlLeft", key: "Control" });
};

// Focus terminal A and check the pointer cursor over it.
await click(a.left + 60, a.top + 40);
await pause(500);
result.steps.cursor = await read(
  `getComputedStyle(document.querySelector("canvas.surface")).cursor`,
);

// Type the marker word into A.
const KEYMAP = { m: "KeyM", a: "KeyA", r: "KeyR", k: "KeyK", e: "KeyE", 9: "Digit9" };
for (const ch of word) await tap(KEYMAP[ch], ch);
await pause(800);

// Drag across the whole line to select the word, then copy. Over-selecting
// the prompt is fine: the assertion only needs the word inside.
await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: a.left + 4, y: a.top + 8 });
await send("Input.dispatchMouseEvent", { type: "mousePressed", x: a.left + 4, y: a.top + 8, button: "left", clickCount: 1 });
for (let i = 1; i <= 6; i++) {
  await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: a.left + 4 + i * 30, y: a.top + 8 });
  await pause(30);
}
await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: a.left + 184, y: a.top + 8, button: "left", clickCount: 1 });
await pause(500);
await chord("KeyC");

let clip = "";
for (let i = 0; i < 20; i++) {
  clip = (await read(`window.__wxr && window.__wxr.clip || ""`)) ?? "";
  if (clip.includes(word)) break;
  await pause(250);
}
result.steps.copied = clip;

// Focus terminal B and paste from the host-owned selection.
const litOf = (id) => `(() => {
  const c = document.getElementById(${JSON.stringify(id)});
  const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
  let lit = 0;
  for (let i = 0; i < d.length; i += 4) {
    if (d[i] + d[i + 1] + d[i + 2] > 350) lit++;
  }
  return lit;
})()`;
// Near the right edge of B: window A is raised and still overlaps the left
// side of B, so a left-side click would land on A.
await click(b.left + b.w - 80, b.top + 40);
await pause(500);
result.steps.b_focused = await read(
  `document.querySelector(".window.focused canvas")?.id ?? "none"`,
);
const before = await read(litOf(b.id));
await chord("KeyV");
await pause(1500);
const after = await read(litOf(b.id));
result.steps.pasted = { before, after };

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

def main [--out: string = "/tmp/webxr-compositor-clipboard.png"]: nothing -> nothing {
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
    ^pkill -x foot | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    log-info $"host on ($HOST_PORT), two foots on ($SOCKET), chromium on ($CDP_PORT)"

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

    let term_a = (job spawn {
        with-env { WAYLAND_DISPLAY: $SOCKET } { ^foot /bin/sh -i | complete | ignore }
    })
    sleep 1sec
    let term_b = (job spawn {
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
        try { job kill $term_a }
        try { job kill $term_b }
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -x foot | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $WORD $out
        | complete
    )
    try { job kill $browser }
    try { job kill $term_a }
    try { job kill $term_b }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
    ^pkill -x foot | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

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
    if not ($steps.mounted? | default false) {
        $failures = ($failures | append "two terminals never appeared together")
    } else {
        if ($steps.cursor? | default "") != "text" {
            $failures = ($failures | append $"the cursor over the terminal is '(($steps.cursor? | default ''))', expected 'text' via cursor-shape")
        }
        if not (($steps.copied? | default "") | str contains $WORD) {
            $failures = ($failures | append $"copy produced '(($steps.copied? | default ''))' on the page instead of text containing ($WORD)")
        }
        let pasted = ($steps.pasted? | default {before: 0, after: 0})
        if (($pasted.after | default 0) - ($pasted.before | default 0)) < 30 {
            $failures = ($failures | append $"Ctrl+Shift+V lit only (($pasted.after | default 0) - ($pasted.before | default 0)) extra pixels in the second terminal")
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  steps     ($steps | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "clipboard flows client to host to page and back into a client"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
