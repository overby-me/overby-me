#!/usr/bin/env nu

# test-browser.nu: run the real host and look at the page it serves.
#
# `cargo test` proves the protocol roundtrips; it cannot say whether the wasm
# mounts, opens the WebSocket, or completes the hello exchange. So this starts
# the host on a test port, drives headless chromium at it, and reads the link
# status off the page.
#
# Usage:
#   just build
#   cargo build --manifest-path host/Cargo.toml
#   nu test-browser.nu                       # -> /tmp/webxr-compositor.png
#
# Exit codes: 0 the page connected to the host · 1 it did not · 2 setup failed.

const HOST_PORT = 8372
const CDP_PORT = 9224
const BUNDLE = "target/dx/webxr-compositor/release/web/public"
const HOST_BIN = "host/target/debug/webxr-compositor"

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset) ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset) ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset) ($msg | str join ' ')" }

# The newest chromium in the store. There is none on PATH in this devshell,
# and pulling one in just for a smoke test is not worth a rebuild.
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

# Deno speaks the DevTools protocol; nushell has no WebSocket client.
const DRIVER = '
const [cdp, url, wait, out] = Deno.args;

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

await send("Page.navigate", { url });

const read = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, returnByValue: true });
  return r.result?.value;
};

// The status flips to "connected:" only after wasm boot, WebSocket open and
// the hello roundtrip, so polling it covers the whole path.
let status = "";
for (let i = 0; i < Number(wait) * 4; i++) {
  status = (await read(
    `document.querySelector("#link-status")?.innerText ?? ""`,
  )) ?? "";
  if (status.startsWith("connected:")) break;
  await pause(250);
}

const shot = await send("Page.captureScreenshot", { format: "png" });
await Deno.writeFile(out, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));

for (const line of lines) console.error("  " + line);
console.log(JSON.stringify({ status, console: lines }));

ws.close();
'

# Enough of a browser to mount a wasm app; nothing here asks for GL yet.
def chromium-args [size: string] {
    [
        "--headless=new"
        "--no-sandbox"
        "--disable-dev-shm-usage"
        "--hide-scrollbars"
        $"--window-size=($size | str replace 'x' ',')"
        $"--remote-debugging-port=($CDP_PORT)"
        "--remote-allow-origins=*"
        "about:blank"
    ]
}

def main [
    --wait: int = 30     # seconds to allow for wasm boot and the hello
    --size: string = "1280x800"
    --out: string = "/tmp/webxr-compositor.png"
]: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    if not ($root | path exists) {
        log-fail $"no bundle at ($root); run `just build` first"
        exit 2
    }
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    if not ($host_bin | path exists) {
        log-fail $"no host at ($host_bin); run `cargo build --manifest-path host/Cargo.toml` first"
        exit 2
    }

    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }

    log-info $"host on ($HOST_PORT), ($chromium | path basename) on ($CDP_PORT)"

    let host = (job spawn {
        with-env {
            WEBXR_COMPOSITOR_LISTEN: $"127.0.0.1:($HOST_PORT)"
            WEBXR_COMPOSITOR_WEB_ROOT: $root
        } { ^$host_bin | complete | ignore }
    })

    mut host_up = false
    for _ in 0..40 {
        let ready = (try {
            http get --max-time 1sec $"http://127.0.0.1:($HOST_PORT)/" | is-not-empty
        } catch { false })
        if $ready { $host_up = true; break }
        sleep 250ms
    }
    if not $host_up {
        job kill $host
        log-fail "the host never answered on its port"
        exit 2
    }

    let args = (chromium-args $size)
    let browser = (job spawn { ^$chromium ...$args | complete | ignore })

    mut cdp_up = false
    for _ in 0..60 {
        let ready = (try {
            http get --max-time 1sec $"http://127.0.0.1:($CDP_PORT)/json/version" | is-not-empty
        } catch { false })
        if $ready { $cdp_up = true; break }
        sleep 250ms
    }
    if not $cdp_up {
        job kill $browser
        job kill $host
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $"($wait)" $out
        | complete
    )
    job kill $browser
    job kill $host

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

    if not (($report.status? | default "") =~ '^connected: webxr-compositor-host') {
        $failures = ($failures | append $"the page never connected; status was '($report.status? | default '')'")
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  status    ($report.status? | default '')"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "the page mounted and completed the hello exchange with the host"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
