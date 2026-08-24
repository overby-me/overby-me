#!/usr/bin/env nu

# test-output.nu: the advertised output mode follows the browser viewport.
#
# Starts the host, confirms the placeholder mode via wayland-info, then lets
# a headless page connect: the page reports its desk size, the host retunes
# the wl_output mode and broadcasts it back, and the header shows the new
# size. A CDP viewport resize must ripple the same way, and a fresh
# wayland-info afterwards must see the browser-chosen mode.
#
# Exit codes: 0 mode follows the viewport · 1 it does not · 2 setup failed.

const HOST_PORT = 8396
const CDP_PORT = 9247
const SOCKET = "wayland-webxr-out"
const BUNDLE = "target/dx/webxr-compositor/release/web/public"
const HOST_BIN = "host/target/debug/webxr-compositor"
const PLACEHOLDER = { width: 1920, height: 1080 }

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

# The current wl_output mode as seen by a real Wayland client.
def output-mode []: nothing -> any {
    let info = (with-env { WAYLAND_DISPLAY: $SOCKET } { ^wayland-info | complete })
    if $info.exit_code != 0 {
        return null
    }
    let parsed = (
        $info.stdout
        | parse --regex `width: (?<width>\d+) px, height: (?<height>\d+) px`
    )
    if ($parsed | is-empty) {
        null
    } else {
        {
            width: ($parsed.0.width | into int)
            height: ($parsed.0.height | into int)
            modes: ($parsed | length)
        }
    }
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

// Desk size plus whether the header status announces exactly that output.
const probe = `(() => {
  const desk = document.getElementById("desk");
  const header = document.querySelector("header")?.textContent ?? "";
  if (!desk) return null;
  const w = desk.clientWidth, h = desk.clientHeight;
  return { w, h, agreed: header.includes("output " + w + "x" + h) };
})()`;

const settle = async () => {
  for (let i = 0; i < 240; i++) {
    const state = await read(probe);
    if (state && state.agreed) return state;
    await pause(250);
  }
  return null;
};

await send("Page.navigate", { url });
const result = {};
result.initial = await settle();
if (result.initial) {
  await send("Emulation.setDeviceMetricsOverride", {
    width: 900,
    height: 600,
    deviceScaleFactor: 1,
    mobile: false,
  });
  for (let i = 0; i < 240; i++) {
    const state = await read(probe);
    if (state && state.agreed && state.w !== result.initial.w) {
      result.resized = state;
      break;
    }
    await pause(250);
  }
}
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

def main []: nothing -> nothing {
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
    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }

    # Reap leftovers of an aborted earlier run; a survivor keeps the port and
    # the fresh host dies at bind.
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    log-info $"host on ($HOST_PORT), chromium on ($CDP_PORT)"

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

    let before = (output-mode)
    if $before == null {
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        log-fail "wayland-info could not read the initial mode"
        exit 2
    }

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
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/"
        | complete
    )
    try { job kill $browser }
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let report = (try { $run.stdout | from json } catch { null })
    if $run.exit_code != 0 or $report == null {
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        log-fail $"the driver failed:\n($run.stdout)\n($run.stderr)"
        exit 2
    }

    # The browser is gone but the mode it chose must persist on the output.
    let after = (output-mode)
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore

    let initial = ($report.initial? | default null)
    let resized = ($report.resized? | default null)
    mut failures = []
    if $before.width != $PLACEHOLDER.width or $before.height != $PLACEHOLDER.height {
        $failures = ($failures | append $"the pre-browser mode was ($before), not the placeholder")
    }
    if $initial == null {
        $failures = ($failures | append "the page never agreed with the host on its viewport")
    } else if $initial.w == $PLACEHOLDER.width and $initial.h == $PLACEHOLDER.height {
        $failures = ($failures | append "the desk happened to match the placeholder; the test proves nothing")
    }
    if $resized == null {
        $failures = ($failures | append "the CDP resize never reached the header status")
    } else {
        if $after == null {
            $failures = ($failures | append "wayland-info could not read the mode after the resize")
        } else {
            if $after.width != $resized.w or $after.height != $resized.h {
                $failures = ($failures | append $"wayland-info sees ($after), the page reported ($resized.w)x($resized.h)")
            }
            if $after.modes != 1 {
                $failures = ($failures | append $"($after.modes) modes advertised; stale ones linger")
            }
        }
    }

    print $"  before   ($before | to json --raw)"
    print $"  initial  ($initial | to json --raw)"
    print $"  resized  ($resized | to json --raw)"
    print $"  after    ($after | to json --raw)"

    if ($failures | is-empty) {
        log-ok "the output mode follows the browser viewport, resize included"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
