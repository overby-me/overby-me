#!/usr/bin/env nu

# test-scale.nu: viewporter and fractional scale are honoured.
#
# Phase A runs the checker with CHECKER_VIEWPORT=1: it displays its 320x240
# buffer through a wp_viewport destination of 160x120, so the page canvas
# must keep the full backing while occupying the half CSS size, pixels
# intact. Phase B runs gnome-calculator in a chromium forced to a 1.25
# devicePixelRatio: the host prefers that fractional scale, GTK re-renders
# at 1.25x with a viewport back to logical, and the canvas backing must be
# about 1.25 times its CSS size.
#
# Exit codes: 0 both honoured · 1 not so · 2 setup failed.

const HOST_PORT = 8386
const CDP_PORT = 9251
const SOCKET = "wayland-webxr-scale"
const BUNDLE = "target/dx/webxr-compositor/release/web/public"
const HOST_BIN = "host/target/debug/webxr-compositor"
const CHECKER_BIN = "host/target/debug/examples/checker"

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

def find-calculator [] {
    let on_path = (which gnome-calculator)
    if ($on_path | is-not-empty) {
        return ($on_path | get 0.path)
    }
    null
}

const DRIVER = '
const [cdp, url] = Deno.args;

const targets = await (await fetch(`http://127.0.0.1:${cdp}/json`)).json();
const page = targets.find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((ok) => ws.onopen = ok);

let next = 1;
const pending = new Map();
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg.result);
    pending.delete(msg.id);
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

// Backing vs CSS geometry of the first window canvas, plus a pixel probe of
// the backing at (10, 10) and the reported devicePixelRatio.
const probe = `(() => {
  const c = document.querySelector(".canvas-holder > canvas.surface");
  if (!c || c.width === 0) return null;
  const r = c.getBoundingClientRect();
  const d = c.getContext("2d").getImageData(10, 10, 1, 1).data;
  return {
    backing: [c.width, c.height],
    css: [Math.round(r.width), Math.round(r.height)],
    pixel: [d[0], d[1], d[2]],
    dpr: window.devicePixelRatio,
  };
})()`;

let state = null;
for (let i = 0; i < 240; i++) {
  state = await read(probe);
  if (state && state.css[0] > 0 && state.css[0] !== state.backing[0]) break;
  await pause(250);
}
console.log(JSON.stringify(state));
ws.close();
'

def chromium-args [scale: string] {
    [
        "--headless=new"
        "--no-sandbox"
        "--disable-dev-shm-usage"
        "--hide-scrollbars"
        $"--force-device-scale-factor=($scale)"
        "--window-size=1280,800"
        $"--remote-debugging-port=($CDP_PORT)"
        "--remote-allow-origins=*"
        "about:blank"
    ]
}

# One phase: host + client + chromium at a device scale, returns the probe.
def run-phase [
    root: string
    host_bin: string
    chromium: string
    scale: string
    client: closure
]: nothing -> any {
    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

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
        return null
    }

    let client_job = (job spawn { do $client })

    let browser = (job spawn { ^$chromium ...(chromium-args $scale) | complete | ignore })
    mut cdp_up = false
    for _ in 0..60 {
        let ready = (try {
            http get --max-time 1sec $"http://127.0.0.1:($CDP_PORT)/json/version" | is-not-empty
        } catch { false })
        if $ready { $cdp_up = true; break }
        sleep 250ms
    }
    let report = (if $cdp_up {
        let run = (
            ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/"
            | complete
        )
        try { $run.stdout | from json } catch { null }
    } else {
        null
    })
    try { job kill $browser }
    try { job kill $client_job }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
    $report
}

def main []: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    let checker_bin = ($env.FILE_PWD | path join $CHECKER_BIN)
    if not ($root | path exists) {
        log-fail $"no bundle at ($root); run `just build` first"
        exit 2
    }
    if not ($host_bin | path exists) or not ($checker_bin | path exists) {
        log-fail "host or checker missing; run the cargo builds first"
        exit 2
    }
    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }
    let calculator = (find-calculator)
    if $calculator == null {
        log-fail "no gnome-calculator on PATH for the fractional phase"
        exit 2
    }

    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $checker_bin | complete | ignore
    ^pkill -f gnome-calculator | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    log-info "phase A: checker through a wp_viewport destination"
    let viewport = (run-phase $root $host_bin $chromium "1" {
        with-env {
            WAYLAND_DISPLAY: $SOCKET
            CHECKER_VIEWPORT: "1"
        } { ^$checker_bin | complete | ignore }
    })

    log-info "phase B: gnome-calculator at devicePixelRatio 1.25"
    let fractional = (run-phase $root $host_bin $chromium "1.25" {
        with-env {
            WAYLAND_DISPLAY: $SOCKET
            GSK_RENDERER: "cairo"
            LIBGL_ALWAYS_SOFTWARE: "1"
        } { ^$calculator | complete | ignore }
    })
    ^pkill -f gnome-calculator | complete | ignore

    mut failures = []
    if $viewport == null {
        $failures = ($failures | append "phase A never produced a scaled canvas")
    } else {
        if $viewport.backing != [320 240] {
            $failures = ($failures | append $"viewport backing is ($viewport.backing), not the full 320x240 buffer")
        }
        if $viewport.css != [160 120] {
            $failures = ($failures | append $"viewport CSS size is ($viewport.css), not the 160x120 destination")
        }
        let pixel = ($viewport.pixel | default [])
        let on_palette = (
            [[229 57 54] [67 160 71] [30 136 229] [253 216 53]]
            | any {|p| $pixel | zip $p | all {|q| ($q.0 - $q.1 | math abs) <= 3 } }
        )
        if not $on_palette {
            $failures = ($failures | append $"viewport pixels are ($pixel), not checker palette")
        }
    }
    if $fractional == null {
        $failures = ($failures | append "phase B never produced a scaled canvas")
    } else {
        if ($fractional.dpr - 1.25 | math abs) > 0.01 {
            $failures = ($failures | append $"the page saw devicePixelRatio ($fractional.dpr), not 1.25")
        }
        let ratio = (($fractional.backing.0 | into float) / ($fractional.css.0 | into float))
        if ($ratio - 1.25 | math abs) > 0.05 {
            $failures = ($failures | append $"calculator backing/CSS ratio is ($ratio), not about 1.25 \(($fractional.backing) vs ($fractional.css))")
        }
    }

    print $"  viewport   ($viewport | to json --raw)"
    print $"  fractional ($fractional | to json --raw)"

    if ($failures | is-empty) {
        log-ok "viewport destinations and fractional scale are honoured"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
