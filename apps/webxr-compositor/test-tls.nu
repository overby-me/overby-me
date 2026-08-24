#!/usr/bin/env nu

# test-tls.nu: the secure transport.
#
# Runs the host in TLS mode with a fixed token and drives the whole stack
# through https/wss in chromium (self-signed, so certificate errors are
# ignored): the page must complete the hello over wss with the token from
# its URL, and a socket presenting a wrong token must be refused.
#
# Exit codes: 0 wss works and bad tokens bounce · 1 not so · 2 setup
# failed.

const HOST_PORT = 8391
const CDP_PORT = 9242
const SOCKET = "wayland-webxr-tls"
const BUNDLE = "target/dx/webxr-compositor/release/web/public"
const HOST_BIN = "host/target/debug/webxr-compositor"
const TOKEN = "secret-for-the-tls-test"

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
const [cdp, url, wssBad, out] = Deno.args;

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
let status = "";
for (let i = 0; i < 120; i++) {
  status = (await read(`document.querySelector("#link-status")?.innerText ?? ""`)) ?? "";
  if (status.startsWith("connected:")) break;
  await pause(250);
}
result.steps.wss = status;

// A wrong token must never reach the open state.
result.steps.rejected = await read(`new Promise((resolve) => {
  const bad = new WebSocket(${JSON.stringify(wssBad)});
  let opened = false;
  bad.onopen = () => { opened = true; bad.close(); };
  bad.onclose = () => resolve(!opened);
  bad.onerror = () => {};
  setTimeout(() => resolve(!opened), 5000);
})`);

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
        "--ignore-certificate-errors"
        "--window-size=1280,800"
        $"--remote-debugging-port=($CDP_PORT)"
        "--remote-allow-origins=*"
        "about:blank"
    ]
}

def main [--out: string = "/tmp/webxr-compositor-tls.png"]: nothing -> nothing {
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

    log-info $"host on wss://($HOST_PORT), chromium on ($CDP_PORT)"

    let host = (job spawn {
        with-env {
            WEBXR_COMPOSITOR_LISTEN: $"127.0.0.1:($HOST_PORT)"
            WEBXR_COMPOSITOR_WEB_ROOT: $root
            WEBXR_COMPOSITOR_WAYLAND_DISPLAY: $SOCKET
            WEBXR_COMPOSITOR_TLS: "1"
            WEBXR_COMPOSITOR_TOKEN: $TOKEN
        } { ^$host_bin | complete | ignore }
    })

    mut host_up = false
    for _ in 0..60 {
        let ready = (try {
            http get --insecure --max-time 1sec $"https://127.0.0.1:($HOST_PORT)/" | is-not-empty
        } catch { false })
        if $ready { $host_up = true; break }
        sleep 250ms
    }
    if not $host_up {
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        log-fail "the host never answered over TLS"
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
        ^deno eval $DRIVER $"($CDP_PORT)" $"https://127.0.0.1:($HOST_PORT)/?token=($TOKEN)" $"ws('s')://127.0.0.1:($HOST_PORT)/ws?token=wrong" $out
        | complete
    )
    try { job kill $browser }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
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
    if not (($steps.wss? | default "") =~ '^connected: webxr-compositor-host') {
        $failures = ($failures | append $"the page never connected over wss; status was '(($steps.wss? | default ''))'")
    }
    if not ($steps.rejected? | default false) {
        $failures = ($failures | append "a wrong token was allowed to open the socket")
    }

    print $"  steps     ($steps | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "wss with the right token works and wrong tokens bounce"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
