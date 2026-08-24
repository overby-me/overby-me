#!/usr/bin/env nu

# test-subsurface.nu: wl_subsurface content is composited.
#
# Runs the bundled subchecker client (a navy parent with an animated child
# subsurface that jumps between two anchor points) and asserts the child
# renders as an overlay: palette pixels, correct offsets including the
# periodic move, and animation, while the parent keeps its own fill.
#
# Exit codes: 0 subsurfaces composite · 1 they do not · 2 setup failed.

const HOST_PORT = 8398
const CDP_PORT = 9249
const SOCKET = "wayland-webxr-sub"
const BUNDLE = "target/dx/webxr-compositor/release/web/public"
const HOST_BIN = "host/target/debug/webxr-compositor"
const CLIENT_BIN = "host/target/debug/examples/subchecker"

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
  const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  return r.result?.value;
};

await send("Page.navigate", { url });

// Parent pixel, sub pixel and the sub offset relative to the parent canvas.
const probe = `(() => {
  const parent = document.querySelector(".canvas-holder > canvas.surface");
  const sub = document.querySelector(".popup canvas.surface");
  if (!parent || parent.width === 0) return null;
  const px = (canvas, x, y) => {
    const d = canvas.getContext("2d").getImageData(x, y, 1, 1).data;
    return [d[0], d[1], d[2]];
  };
  const state = { parent: px(parent, 10, 10), pw: parent.width, ph: parent.height };
  if (sub && sub.width > 0) {
    const pr = parent.getBoundingClientRect();
    const sr = sub.getBoundingClientRect();
    state.sub = px(sub, 60, 45);
    state.sw = sub.width;
    state.sh = sub.height;
    state.offset = [Math.round(sr.left - pr.left), Math.round(sr.top - pr.top)];
  }
  return state;
})()`;

const result = { steps: {}, console: lines };
let state = null;
for (let i = 0; i < 240; i++) {
  state = await read(probe);
  // A fresh canvas is 300x150 until the first frame sizes it; wait that out.
  if (state && state.sub && state.sw === 120) break;
  await pause(250);
}
result.steps.mounted = state;
if (!state || !state.sub) {
  console.log(JSON.stringify(result));
  ws.close();
  Deno.exit(0);
}

// The child moves every ~0.75s and recolours every ~0.5s: watch for both
// anchor points and at least two palette colours.
const offsets = new Set();
const colors = new Set();
for (let i = 0; i < 40; i++) {
  const now = await read(probe);
  if (now && now.sub) {
    offsets.add(now.offset.join("x"));
    colors.add(now.sub.join(","));
  }
  await pause(250);
}
result.steps.offsets = [...offsets];
result.steps.colors = [...colors];

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

# True when an "r,g,b" sample is within tolerance of the expectation.
def close-to [sample: string, expected: list<int>] {
    let got = ($sample | split row "," | each {|v| $v | into int })
    ($got | zip $expected | all {|pair| ($pair.0 - $pair.1 | math abs) <= 3 })
}

def main [--out: string = "/tmp/webxr-compositor-subsurface.png"]: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    let client_bin = ($env.FILE_PWD | path join $CLIENT_BIN)
    if not ($root | path exists) {
        log-fail $"no bundle at ($root); run `just build` first"
        exit 2
    }
    if not ($host_bin | path exists) or not ($client_bin | path exists) {
        log-fail "host or subchecker missing; run the cargo builds first"
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
    ^pkill -f $client_bin | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    log-info $"host on ($HOST_PORT), subchecker on ($SOCKET), chromium on ($CDP_PORT)"

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

    let client = (job spawn {
        with-env { WAYLAND_DISPLAY: $SOCKET } { ^$client_bin | complete | ignore }
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
        try { job kill $client }
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f $client_bin | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $out
        | complete
    )
    try { job kill $browser }
    try { job kill $client }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $client_bin | complete | ignore
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
    let mounted = ($steps.mounted? | default null)
    mut failures = []
    if $mounted == null or ($mounted.sub? | default null) == null {
        $failures = ($failures | append "the subsurface overlay never appeared")
    } else {
        if not (close-to ($mounted.parent | str join ",") [16 32 48]) {
            $failures = ($failures | append $"the parent fill is ($mounted.parent), not navy")
        }
        if $mounted.sw != 120 or $mounted.sh != 90 {
            $failures = ($failures | append $"the subsurface canvas is ($mounted.sw)x($mounted.sh), not 120x90")
        }
        let palette = [[229 57 54] [67 160 71] [30 136 229] [253 216 53]]
        let colors = ($steps.colors? | default [])
        let on_palette = ($colors | where {|c| $palette | any {|p| close-to $c $p } })
        if ($on_palette | length) < 2 {
            $failures = ($failures | append $"only (($on_palette | length)) palette colours seen in ($colors); the child never animated")
        }
        let offsets = ($steps.offsets? | default [])
        for spot in ["60x40" "140x90"] {
            if not ($spot in $offsets) {
                $failures = ($failures | append $"the subsurface was never seen at ($spot); offsets were ($offsets)")
            }
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  steps     ($steps | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "subsurfaces composite, move and animate as overlays"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
