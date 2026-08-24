#!/usr/bin/env nu

# test-video.nu: sustained motion switches a surface to H.264.
#
# Runs the checker in fast mode (a full-surface repaint every frame): the
# host must flip it into video mode, the page must decode a stream of
# VideoFrames through WebCodecs, and the canvas must still show the
# checker palette, within codec tolerance. The encoded stream must also
# be far smaller than the raw frames it replaces.
#
# Exit codes: 0 video mode engages and decodes correctly · 1 it does not ·
# 2 setup failed.

const HOST_PORT = 8393
const CDP_PORT = 9244
const SOCKET = "wayland-webxr-video"
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
  const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  return r.result?.value;
};

await send("Page.navigate", { url });

const result = { steps: {}, console: lines };
let mounted = null;
for (let i = 0; i < 240; i++) {
  mounted = await read(`(() => {
    const c = document.querySelector("canvas.surface");
    return c && c.width > 0 ? { w: c.width, h: c.height } : null;
  })()`);
  if (mounted) break;
  await pause(250);
}
if (!mounted) {
  console.log(JSON.stringify(result));
  ws.close();
  Deno.exit(0);
}
result.steps.mounted = mounted;

// Wait until the host has flipped to video and a real stream decoded.
let stats = null;
for (let i = 0; i < 80; i++) {
  await pause(250);
  stats = await read(
    `window.__wxr ? {
      videoFrames: window.__wxr.videoFrames || 0,
      videoBytes: window.__wxr.videoBytes || 0,
      videoRaw: window.__wxr.videoRaw || 0,
    } : null`,
  );
  if (stats && stats.videoFrames > 60) break;
}
result.steps.video = stats;
await pause(300);

// The decoded picture must still be the checker, within codec tolerance.
result.steps.picture = await read(`(() => {
  const c = document.querySelector("canvas.surface");
  const ctx = c.getContext("2d");
  const px = (x, y) => Array.from(ctx.getImageData(x, y, 1, 1).data).slice(0, 3);
  const palette = [[229,57,54],[67,160,71],[30,136,229],[253,216,53]];
  const spots = [px(80, 60), px(240, 60), px(80, 180), px(240, 180)];
  let matched = 0;
  const seen = new Set();
  for (const [r, g, b] of spots) {
    for (let i = 0; i < palette.length; i++) {
      const [pr, pg, pb] = palette[i];
      if (Math.abs(r-pr) < 60 && Math.abs(g-pg) < 60 && Math.abs(b-pb) < 60) {
        matched++;
        seen.add(i);
        break;
      }
    }
  }
  return { matched, distinct: seen.size, spots };
})()`);

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

def main [--out: string = "/tmp/webxr-compositor-video.png"]: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    let checker_bin = ($env.FILE_PWD | path join $CHECKER_BIN)
    if not ($root | path exists) {
        log-fail $"no bundle at ($root); run `just build` first"
        exit 2
    }
    if not ($host_bin | path exists) or not ($checker_bin | path exists) {
        log-fail "missing host or checker; run `just video` instead"
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

    log-info $"host on ($HOST_PORT), fast checker on ($SOCKET), chromium on ($CDP_PORT)"

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
        with-env { WAYLAND_DISPLAY: $SOCKET, CHECKER_FAST: "1" } {
            ^$checker_bin | complete | ignore
        }
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
    if ($steps.mounted? | default null) == null {
        $failures = ($failures | append "the checker window never appeared")
    } else {
        let video = ($steps.video? | default null)
        if $video == null or ($video.videoFrames | default 0) < 60 {
            $failures = ($failures | append $"only (($video.videoFrames? | default 0)) video frames decoded; the host never entered video mode")
        } else {
            let ratio = (($video.videoRaw | default 0) / (($video.videoBytes | default 1)))
            print $"  video     ($video.videoFrames) frames, ($video.videoBytes) wire for ($video.videoRaw) raw, ratio ($ratio | into int)x"
            if $ratio < 10 {
                $failures = ($failures | append $"the encoded stream is only ($ratio | into int)x smaller than raw")
            }
        }
        let picture = ($steps.picture? | default {matched: 0, distinct: 0})
        if ($picture.matched | default 0) < 4 or ($picture.distinct | default 0) < 4 {
            $failures = ($failures | append $"decoded quadrants matched (($picture.matched | default 0)) palette colours, (($picture.distinct | default 0)) distinct; decode is wrong: (($picture.spots? | default [] | to json --raw))")
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  steps     ($steps | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "sustained motion streams as H.264 and decodes correctly"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
