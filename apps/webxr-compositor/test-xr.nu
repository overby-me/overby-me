#!/usr/bin/env nu

# test-xr.nu: the 3D mode renders real window content.
#
# Runs the checker client, flips the page into the 3D view and samples the
# WebGL canvas (drawn with preserveDrawingBuffer for exactly this): the
# checker palette must appear on the projected quad, and toggling back must
# restore the flat desk. The immersive XR session itself needs a headset
# and shares this scene code; the preview is what a machine can prove.
#
# Exit codes: 0 the 3D scene shows the app · 1 it does not · 2 setup
# failed.

const HOST_PORT = 8388
const CDP_PORT = 9239
const SOCKET = "wayland-webxr-3d"
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

let flat = null;
for (let i = 0; i < 240; i++) {
  flat = await read(`(() => {
    const c = document.querySelector(".window canvas.surface");
    return c && c.width > 0 ? { w: c.width } : null;
  })()`);
  if (flat) break;
  await pause(250);
}
const result = { steps: {}, console: lines };
if (!flat) {
  console.log(JSON.stringify(result));
  ws.close();
  Deno.exit(0);
}
result.steps.flat = true;
await pause(500);

// Into the 3D view.
const toggle = await read(`(() => {
  const b = document.getElementById("toggle-3d");
  if (!b) return null;
  const r = b.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
})()`);
await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: toggle.x, y: toggle.y });
await send("Input.dispatchMouseEvent", { type: "mousePressed", x: toggle.x, y: toggle.y, button: "left", clickCount: 1 });
await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: toggle.x, y: toggle.y, button: "left", clickCount: 1 });
await pause(2000);

// Sample the WebGL frame through a 2D canvas.
const SAMPLE = `(() => {
  const xr = document.getElementById("xr-canvas");
  if (!xr || xr.width === 0) return null;
  const t = document.createElement("canvas");
  t.width = xr.width; t.height = xr.height;
  const ctx = t.getContext("2d");
  ctx.drawImage(xr, 0, 0);
  const d = ctx.getImageData(0, 0, t.width, t.height).data;
  const palette = [[229,57,54],[67,160,71],[30,136,229],[253,216,53]];
  let lit = 0, matched = 0;
  for (let i = 0; i < d.length; i += 4) {
    const r = d[i], g = d[i+1], b = d[i+2];
    if (r + g + b > 60) lit++;
    for (const [pr, pg, pb] of palette) {
      if (Math.abs(r-pr) < 45 && Math.abs(g-pg) < 45 && Math.abs(b-pb) < 45) { matched++; break; }
    }
  }
  return { w: xr.width, h: xr.height, lit, matched };
})()`;
let sample = null;
for (let i = 0; i < 40; i++) {
  sample = await read(SAMPLE);
  if (sample && sample.matched > 500) break;
  await pause(250);
}
result.steps.scene = sample;

const shot = await send("Page.captureScreenshot", { format: "png" });
await Deno.writeFile(out, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));

// And back to the flat desk.
await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: toggle.x, y: toggle.y });
await send("Input.dispatchMouseEvent", { type: "mousePressed", x: toggle.x, y: toggle.y, button: "left", clickCount: 1 });
await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: toggle.x, y: toggle.y, button: "left", clickCount: 1 });
await pause(800);
result.steps.back_flat = await read(`(() =>
  document.getElementById("xr-canvas") === null &&
  getComputedStyle(document.getElementById("desk")).visibility === "visible"
)()`);

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

def main [--out: string = "/tmp/webxr-compositor-3d.png"]: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    let checker_bin = ($env.FILE_PWD | path join $CHECKER_BIN)
    if not ($root | path exists) {
        log-fail $"no bundle at ($root); run `just build` first"
        exit 2
    }
    if not ($host_bin | path exists) or not ($checker_bin | path exists) {
        log-fail "missing host or checker; run `just xr` instead"
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
    if not $cdp_up {
        try { job kill $browser }
        try { job kill $checker }
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f $checker_bin | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
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
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $checker_bin | complete | ignore
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
    if not ($steps.flat? | default false) {
        $failures = ($failures | append "the flat view never showed the checker")
    } else {
        let scene = ($steps.scene? | default null)
        if $scene == null {
            $failures = ($failures | append "the 3D canvas never appeared")
        } else {
            if ($scene.matched | default 0) < 500 {
                $failures = ($failures | append $"only (($scene.matched | default 0)) checker-palette pixels in the 3D scene; the texture never made it onto the quad")
            }
        }
        if not ($steps.back_flat? | default false) {
            $failures = ($failures | append "toggling back did not restore the flat desk")
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  steps     ($steps | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "the 3D scene renders live window content and toggles back"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
