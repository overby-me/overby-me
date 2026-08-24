#!/usr/bin/env nu

# test-surface.nu: prove pixels travel wayland client → host → browser canvas.
#
# Starts the host, the deterministic `checker` example client on its Wayland
# socket, and headless chromium on its page; then samples the four quadrant
# pixels of the window canvas twice. The first sample proves the shm pipeline
# (exact palette colours); the second proves animation (the quadrants
# rotated between samples).
#
# Exit codes: 0 pixels flowed and animated · 1 they did not · 2 setup failed.

const HOST_PORT = 8374
const CDP_PORT = 9225
const SOCKET = "wayland-webxr-surface"
const BUNDLE = "target/dx/webxr-compositor/release/web/public"
const HOST_BIN = "host/target/debug/webxr-compositor"
const CHECKER_BIN = "host/target/debug/examples/checker"

# The checker's palette as putImageData leaves it: r,g,b strings.
const PALETTE = ["229,57,54" "67,160,71" "30,136,229" "253,216,53"]

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

// Quadrant centres of the 320x240 checker window.
const probe = `(() => {
  const c = document.querySelector("canvas.surface");
  if (!c || c.width === 0) return null;
  const ctx = c.getContext("2d");
  const px = (x, y) => Array.from(ctx.getImageData(x, y, 1, 1).data).slice(0, 3).join(",");
  return { w: c.width, h: c.height, q: [px(80, 60), px(240, 60), px(80, 180), px(240, 180)] };
})()`;

let first = null;
for (let i = 0; i < 240; i++) {
  first = await read(probe);
  if (first) break;
  await pause(250);
}

let second = null;
let stats = null;
if (first) {
  await pause(1200);
  second = await read(probe);
  stats = await read(
    `window.__wxr ? { frames: window.__wxr.frames, wire: window.__wxr.bytes, raw: window.__wxr.raw } : null`,
  );
}

const shot = await send("Page.captureScreenshot", { format: "png" });
await Deno.writeFile(out, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));

for (const line of lines) console.error("  " + line);
console.log(JSON.stringify({ first, second, stats, console: lines }));
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

def main [--out: string = "/tmp/webxr-compositor-surface.png"]: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    let checker_bin = ($env.FILE_PWD | path join $CHECKER_BIN)
    for needed in [[$root "just build"] [$host_bin "cargo build --manifest-path host/Cargo.toml"] [$checker_bin "cargo build --manifest-path host/Cargo.toml --example checker"]] {
        if not (($needed | get 0) | path exists) {
            log-fail $"missing (($needed | get 0)); run `(($needed | get 1))` first"
            exit 2
        }
    }

    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }

    # Reap leftovers of an aborted earlier run; a survivor keeps the port and
    # the fresh host dies at bind.
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $checker_bin | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    log-info $"host on ($HOST_PORT), checker on ($SOCKET), chromium on ($CDP_PORT)"

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

    let checker = (job spawn {
        with-env { WAYLAND_DISPLAY: $SOCKET } { ^$checker_bin | complete | ignore }
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
    def reap [host_bin: string, checker_bin: string, cdp: int] {
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f $checker_bin | complete | ignore
        ^pkill -f $"remote-debugging-port=($cdp)" | complete | ignore
    }

    if not $cdp_up {
        try { job kill $browser }
        try { job kill $checker }
        try { job kill $host }
        reap $host_bin $checker_bin $CDP_PORT
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $out
        | complete
    )
    try { job kill $browser }
    try { job kill $checker }
    try { job kill $host }
    reap $host_bin $checker_bin $CDP_PORT

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
    let first = ($report.first? | default null)
    if $first == null {
        $failures = ($failures | append "no window canvas ever got pixels")
    } else {
        if $first.w != 320 or $first.h != 240 {
            $failures = ($failures | append $"canvas is ($first.w)x($first.h), expected 320x240")
        }
        for q in $first.q {
            if not ($q in $PALETTE) {
                $failures = ($failures | append $"quadrant colour ($q) is not in the checker palette")
            }
        }
        let second = ($report.second? | default null)
        if $second == null {
            $failures = ($failures | append "no second sample")
        } else if ($second.q == $first.q) {
            $failures = ($failures | append "the quadrants never rotated; frame callbacks are not driving the client")
        }
        let stats = ($report.stats? | default null)
        if $stats == null {
            $failures = ($failures | append "the page recorded no frame stats")
        } else {
            let wire = ($stats.wire | default 0)
            let raw = ($stats.raw | default 0)
            if $wire == 0 or ($raw / $wire) < 20 {
                $failures = ($failures | append $"solid frames travelled at ($wire) wire bytes for ($raw) raw; compression is not engaging")
            } else {
                print $"  wire      ($wire) bytes for ($raw) raw, ratio (($raw / $wire))x"
            }
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  first     ($first | default {} | to json --raw)"
    print $"  second    ($report.second? | default {} | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "shm pixels reached the canvas and animated"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
