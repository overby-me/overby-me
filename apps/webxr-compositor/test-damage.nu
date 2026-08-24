#!/usr/bin/env nu

# test-damage.nu: prove typing ships damage rects, not full frames.
#
# Same rig as test-input.nu, but the assertion is about payload size: the
# page counts every Frame message on window.__wxr, and the average frame
# that typing produces must be a small fraction of the full surface
# (696x468x4 is about 1.3 MB; an echoed glyph row is tens of KB).
#
# Exit codes: 0 damage stays small · 1 frames are full-size · 2 setup failed.

const HOST_PORT = 8378
const CDP_PORT = 9229
const SOCKET = "wayland-webxr-damage"
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

const cx = geo.left + 60;
const cy = geo.top + 40;
await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: cx, y: cy });
await send("Input.dispatchMouseEvent", { type: "mousePressed", x: cx, y: cy, button: "left", clickCount: 1 });
await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: cx, y: cy, button: "left", clickCount: 1 });
await pause(500);

const stats = `window.__wxr ? { frames: window.__wxr.frames, bytes: window.__wxr.bytes, lastW: window.__wxr.lastW, lastH: window.__wxr.lastH } : null`;
const before = await read(stats);

const keys = [
  ["KeyD", "d"], ["KeyA", "a"], ["KeyM", "m"], ["KeyA", "a"], ["KeyG", "g"],
  ["KeyE", "e"], ["Space", " "], ["KeyT", "t"], ["KeyE", "e"], ["KeyS", "s"], ["KeyT", "t"],
];
for (const [code, key] of keys) {
  await send("Input.dispatchKeyEvent", { type: "keyDown", code, key });
  await send("Input.dispatchKeyEvent", { type: "keyUp", code, key });
  await pause(60);
}
await pause(1200);

const after = await read(stats);

const shot = await send("Page.captureScreenshot", { format: "png" });
await Deno.writeFile(out, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));

for (const line of lines) console.error("  " + line);
console.log(JSON.stringify({ mounted: true, geo, before, after, console: lines }));
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

def main [--out: string = "/tmp/webxr-compositor-damage.png"]: nothing -> nothing {
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
        ^pkill -x foot | complete | ignore
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

    mut failures = []
    if not ($report.mounted? | default false) {
        $failures = ($failures | append "the terminal window never appeared on the page")
    } else {
        let geo = ($report.geo? | default {w: 0, h: 0})
        let full_bytes = (($geo.w | default 0) * ($geo.h | default 0) * 4)
        let before = ($report.before? | default {frames: 0, bytes: 0})
        let after = ($report.after? | default null)
        if $after == null {
            $failures = ($failures | append "the page never recorded frame stats")
        } else {
            let frames = (($after.frames | default 0) - ($before.frames | default 0))
            let bytes = (($after.bytes | default 0) - ($before.bytes | default 0))
            if $frames < 3 {
                $failures = ($failures | append $"typing produced only ($frames) frames; the echo never arrived")
            } else {
                let avg = ($bytes / $frames)
                print $"  frames    ($frames), avg ($avg | into int) bytes vs full ($full_bytes)"
                if ($avg * 4) > $full_bytes {
                    $failures = ($failures | append $"average typed frame is ($avg | into int) bytes, more than a quarter of the full ($full_bytes); damage cropping is not working")
                }
                if (($after.lastH | default 0) * 2) > ($geo.h | default 0) {
                    $failures = ($failures | append $"the last damage rect spans (($after.lastH | default 0)) rows of (($geo.h | default 0)); expected a glyph-row patch")
                }
            }
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "typing ships small damage rects, not full frames"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
