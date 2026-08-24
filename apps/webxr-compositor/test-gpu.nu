#!/usr/bin/env nu

# test-gpu.nu: hardware clients render through dmabuf readback.
#
# Runs es2gears_wayland (real GPU via EGL) against the host with the GL
# runtime wired up, captures the host log, and asserts the frames actually
# took the dmabuf path (trace lines), produced no readback errors, animate
# in the browser, and stream as video. Exits 2 when the machine has no
# usable render node; software rendering hosts are simply not covered.
#
# Exit codes: 0 dmabuf readback works · 1 it is broken · 2 setup failed or
# no GPU.

const HOST_PORT = 8395
const CDP_PORT = 9246
const SOCKET = "wayland-webxr-hw"
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
await pause(1000);

const sample = `(() => {
  const c = document.querySelector("canvas.surface");
  const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
  let sum = 0;
  for (let i = 0; i < d.length; i += 97) sum += d[i];
  return sum;
})()`;
const first = await read(sample);
await pause(800);
const second = await read(sample);
result.steps.animated = first !== second;

let stats = null;
for (let i = 0; i < 40; i++) {
  stats = await read(`window.__wxr ? { videoFrames: window.__wxr.videoFrames || 0 } : null`);
  if (stats && stats.videoFrames > 30) break;
  await pause(250);
}
result.steps.video = stats;

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

def main [--out: string = "/tmp/webxr-compositor-gpu.png"]: nothing -> nothing {
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
    if not ("/dev/dri" | path exists) {
        log-fail "no /dev/dri; this machine cannot cover the GPU path"
        exit 2
    }
    let glvnd = (^nix build --no-link --print-out-paths "nixpkgs#libglvnd" | complete)
    let demos = (^nix build --no-link --print-out-paths "nixpkgs#mesa-demos" | complete)
    if $glvnd.exit_code != 0 or $demos.exit_code != 0 {
        log-fail "could not resolve libglvnd or mesa-demos from nixpkgs"
        exit 2
    }
    let glvnd_lib = $"($glvnd.stdout | str trim)/lib"
    let gears = $"($demos.stdout | str trim)/bin/es2gears_wayland"
    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }

    # Reap leftovers of an aborted earlier run; a survivor keeps the port and
    # the fresh host dies at bind.
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f es2gears_wayland | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }
    let host_log = (mktemp --suffix .webxr-gpu.log)

    log-info $"host on ($HOST_PORT), gears on ($SOCKET), chromium on ($CDP_PORT)"

    let host = (job spawn {
        with-env {
            WEBXR_COMPOSITOR_LISTEN: $"127.0.0.1:($HOST_PORT)"
            WEBXR_COMPOSITOR_WEB_ROOT: $root
            WEBXR_COMPOSITOR_WAYLAND_DISPLAY: $SOCKET
            RUST_LOG: "webxr_compositor=trace"
            LD_LIBRARY_PATH: $glvnd_lib
            __EGL_VENDOR_LIBRARY_DIRS: "/run/opengl-driver/share/glvnd/egl_vendor.d"
        } { ^$host_bin out+err> $host_log }
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
    sleep 500ms
    if not (open $host_log | str contains "dmabuf readback ready") {
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        log-fail "no usable render node; the GPU path cannot be covered here"
        exit 2
    }

    let gears_job = (job spawn {
        with-env { WAYLAND_DISPLAY: $SOCKET } { ^$gears | complete | ignore }
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
        try { job kill $gears_job }
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f es2gears_wayland | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $out
        | complete
    )
    try { job kill $browser }
    try { job kill $gears_job }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f es2gears_wayland | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    if $run.exit_code != 0 {
        rm -f $host_log
        log-fail $"the driver failed:\n($run.stderr)"
        exit 2
    }
    let report = (try { $run.stdout | from json } catch { null })
    if $report == null {
        rm -f $host_log
        log-fail $"the driver said nothing useful:\n($run.stdout)\n($run.stderr)"
        exit 2
    }

    let log_text = (open $host_log)
    rm -f $host_log
    let readbacks = ($log_text | lines | where {|l| $l =~ 'dmabuf readback$' } | length)
    let errors = ($log_text | lines | where {|l| $l =~ "gpu readback" } | length)

    let steps = ($report.steps? | default {})
    mut failures = []
    if ($steps.mounted? | default null) == null {
        $failures = ($failures | append "the gears window never appeared")
    } else {
        if $readbacks < 30 {
            $failures = ($failures | append $"only ($readbacks) dmabuf readbacks; the client fell back to software")
        }
        if $errors > 0 {
            $failures = ($failures | append $"($errors) gpu readback errors in the host log")
        }
        if not ($steps.animated? | default false) {
            $failures = ($failures | append "the gears never animated on the canvas")
        }
        let video = ($steps.video? | default null)
        if $video == null or ($video.videoFrames | default 0) < 30 {
            $failures = ($failures | append "the GPU stream never entered video mode")
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  readbacks ($readbacks), errors ($errors)"
    print $"  steps     ($steps | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "hardware frames flow through dmabuf readback and stream as video"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
