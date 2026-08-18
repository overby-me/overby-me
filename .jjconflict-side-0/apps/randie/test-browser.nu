#!/usr/bin/env nu

# test-browser.nu: run the built bundle in a real browser and look at it.
#
# `cargo test` proves the firmware navigates and the room is what the map says.
# It cannot say whether the page mounts, whether the canvas gets a context, or
# whether anything is drawn on it, and those are exactly the ways a wasm
# frontend breaks. So this serves the bundle, drives headless chromium at it,
# reads the canvas back, and checks that the room, the beams and the clock are
# all there.
#
# Usage:
#   just build
#   nu test-browser.nu                       # -> /tmp/randsim.png
#   nu test-browser.nu --wait 10 --size 1600x1000
#
# Exit codes: 0 the simulator is running and drawing · 1 it is not · 2 setup
# failed.
#
# The canvas is read back with `getImageData` rather than measured off the
# screenshot, because a 2D canvas can be read back directly and counting
# coloured pixels says which parts are missing: no black is a room that did not
# load, no green is sensors that are not being cast, no red is a minimap that
# never drew. The screenshot is saved anyway, to be looked at when one of those
# fails.
#
# Deno is embedded below rather than kept in a file of its own because nushell
# has neither an HTTP server nor a WebSocket client, and both are needed: the
# bundle has to be served over HTTP for the wasm to instantiate, and the
# DevTools protocol is a WebSocket.

const PORT = 8931
const CDP_PORT = 9223
const BUNDLE = "target/dx/randie/release/web/public"

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset) ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset) ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset) ($msg | str join ' ')" }

# The newest chromium in the store. There is none on PATH in this devshell, and
# pulling one in just for a smoke test is not worth a rebuild.
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
const [root, port, cdp, wait, out] = Deno.args;
const types = {
  ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm",
  ".css": "text/css", ".png": "image/png", ".ico": "image/x-icon",
};
const server = Deno.serve(
  { port: Number(port), hostname: "127.0.0.1", onListen: () => {} },
  async (req) => {
    const path = decodeURIComponent(new URL(req.url).pathname);
    for (const p of [path, "/index.html"]) {
      try {
        const file = await Deno.readFile(root + p);
        const type = types[p.slice(p.lastIndexOf("."))] ?? "application/octet-stream";
        return new Response(file, { headers: { "content-type": type } });
      } catch { /* fall through to index.html */ }
    }
    return new Response("not found", { status: 404 });
  },
);

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

await send("Page.navigate", { url: `http://127.0.0.1:${port}/` });

// Fetching and instantiating the wasm takes a moment, and everything measured
// below is blank until the canvas is on the page.
let mounted = false;
for (let i = 0; i < 120; i++) {
  const r = await send("Runtime.evaluate", {
    expression: "!!document.querySelector('#room')",
    returnByValue: true,
  });
  if (r.result && r.result.value) { mounted = true; break; }
  await pause(250);
}
await pause(Number(wait) * 1000);

const probe = `(() => {
  const canvas = document.querySelector("#room");
  if (!canvas) return { mounted: false };
  const ctx = canvas.getContext("2d");
  const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
  let white = 0, black = 0, green = 0, blue = 0, red = 0;
  for (let i = 0; i < data.length; i += 4) {
    const r = data[i], g = data[i + 1], b = data[i + 2];
    if (r > 240 && g > 240 && b > 240) white++;
    else if (r < 40 && g < 40 && b < 40) black++;
    else if (g > 100 && r < 120 && b < 120) green++;
    else if (b > 150 && r < 80 && g < 80) blue++;
    else if (r > 150 && g < 80 && b < 80) red++;
  }
  const panel = document.querySelector(".panel");
  const text = panel ? panel.innerText : "";
  const seconds = /Time\\s+([0-9.]+) s/.exec(text);
  const task = /Task\\s+(.+)/.exec(text);
  return {
    mounted: true,
    width: canvas.width, height: canvas.height,
    white, black, green, blue, red,
    seconds: seconds ? Number(seconds[1]) : 0,
    task: task ? task[1].trim() : "",
  };
})()`;

const result = await send("Runtime.evaluate", { expression: probe, returnByValue: true });

const shot = await send("Page.captureScreenshot", { format: "png" });
await Deno.writeFile(out, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));

for (const line of lines) console.error("  " + line);
console.log(JSON.stringify({ mounted, ...(result.result?.value ?? {}), console: lines }));

ws.close();
await server.shutdown();
'

# Enough of a browser to mount a wasm app and draw on a 2D canvas. No GL flags:
# nothing here asks for a GL context.
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
    --wait: int = 6      # seconds to let the simulation run before looking
    --size: string = "1280x800"
    --out: string = "/tmp/randsim.png"
]: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    if not ($root | path exists) {
        log-fail $"no bundle at ($root); run `just build` first"
        exit 2
    }

    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }

    log-info $"($chromium | path basename) on ($CDP_PORT), bundle on ($PORT)"

    let args = (chromium-args $size)
    let browser = (job spawn { ^$chromium ...$args | complete | ignore })

    mut up = false
    for _ in 0..60 {
        let ready = (try {
            http get --max-time 1sec $"http://127.0.0.1:($CDP_PORT)/json/version" | is-not-empty
        } catch { false })
        if $ready { $up = true; break }
        sleep 250ms
    }

    if not $up {
        job kill $browser
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (^deno eval $DRIVER $root $"($PORT)" $"($CDP_PORT)" $"($wait)" $out | complete)
    job kill $browser

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

    if not $report.mounted {
        $failures = ($failures | append "the canvas never appeared")
    }
    if ($report.black? | default 0) < 100 {
        $failures = ($failures | append "no walls were drawn")
    }
    if ($report.white? | default 0) < 100 {
        $failures = ($failures | append "no floor was drawn")
    }
    if ($report.green? | default 0) < 10 {
        $failures = ($failures | append "no sensor beams were drawn")
    }
    if ($report.red? | default 0) < 1 {
        $failures = ($failures | append "nothing was drawn on the minimap")
    }
    if ($report.seconds? | default 0.0) <= 0.0 {
        $failures = ($failures | append "the clock never started")
    }

    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  canvas    ($report.width? | default 0)x($report.height? | default 0)"
    print $"  pixels    floor ($report.white? | default 0), walls ($report.black? | default 0), beams ($report.green? | default 0), glass ($report.blue? | default 0), marks ($report.red? | default 0)"
    print $"  clock     ($report.seconds? | default 0) s of simulated flight"
    print $"  drone     ($report.task? | default 'unknown')"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "the simulator mounted, drew the room, and flew the drone"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
