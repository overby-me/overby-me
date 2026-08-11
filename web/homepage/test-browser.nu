#!/usr/bin/env nu

# test-browser.nu — render a screensaver in a real browser and look at it.
#
# The 2D savers can be checked with `just shot`, because they rasterise on the
# CPU and the crate's `render` example can run them with no browser at all. The
# Shadertoy savers cannot: they are fragment shaders, and the only thing that
# can run one is a GL driver. So this serves the built bundle, drives headless
# chromium at it, and saves a PNG of each saver.
#
# Usage:
#   just build-whole                  # or `just build`
#   nu test-browser.nu starnest       # -> /tmp/screensaver-starnest.png
#   nu test-browser.nu starnest --wait 8 --size 1280x720
#   nu test-browser.nu --all          # every Shadertoy saver, into a montage
#   nu test-browser.nu skyline --console --query "scale=0.25"
#
# Exit codes: 0 the canvas has a picture on it · 1 it does not · 2 setup failed.
#
# Why the DevTools protocol rather than `chromium --screenshot`: the screenshot
# flag wants `--virtual-time-budget`, which runs the clock as fast as it can and
# stops it again, and a saver's animation frames get starved. It reports a black
# canvas for a shader that renders perfectly well, and which shaders it lies
# about changes from run to run. Driving the browser and letting it have real
# seconds does not.
#
# Deno appears twice below, embedded rather than in files of its own, because
# nushell has neither an HTTP server nor a WebSocket client and both are needed:
# the client-side router wants a server with an index.html fallback, and the
# DevTools protocol is a WebSocket.

const PORT = 8137
const CDP_PORT = 9222

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset) ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset) ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset) ($msg | str join ' ')" }

# The newest chromium in the store. There is no chromium on PATH in this
# devshell, and pulling one in just for a screenshot is not worth a rebuild.
def find-chromium [] {
    let found = (
        ls /nix/store
        | where name =~ '-chromium-[0-9]'
        | where type == dir
        | get name
        | each {|d| $"($d)/bin/chromium" }
        | where {|p| ($p | path exists) }
        | sort
    )
    if ($found | is-empty) { null } else { $found | last }
}

# Serve the bundle, then drive the browser at it and save a PNG per saver.
#
# The fallback to index.html is what makes a client-side route like
# /screensaver/starnest load at all. `Page.captureScreenshot` is used rather
# than reading the canvas back with toDataURL, because a WebGL canvas without
# preserveDrawingBuffer is blank to toDataURL outside the frame that drew it,
# and turning that on for the sake of a test would slow down the real thing.
const DRIVER = '
const [root, port, cdp, wait, out, ...slugs] = Deno.args;
const types = {
  ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm",
  ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml",
  ".json": "application/json", ".woff2": "font/woff2", ".ico": "image/x-icon",
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
const console_lines = [];
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg.result);
    pending.delete(msg.id);
  } else if (msg.method === "Runtime.consoleAPICalled") {
    console_lines.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
  } else if (msg.method === "Runtime.exceptionThrown") {
    console_lines.push("EXCEPTION " + JSON.stringify(msg.params.exceptionDetails.text));
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

for (const slug of slugs) {
  console_lines.length = 0;
  await send("Page.navigate", { url: `http://127.0.0.1:${port}/screensaver/${slug}` });
  // Wait for the stage to mount before starting the clock. Fetching and
  // instantiating the wasm takes a while, and a screenshot taken before that is
  // the browser default white rather than anything the saver did.
  for (let i = 0; i < 120; i++) {
    const r = await send("Runtime.evaluate", {
      expression: "!!document.querySelector('#screensaver-canvas')",
      returnByValue: true,
    });
    if (r.result && r.result.value) break;
    await pause(250);
  }
  await pause(Number(wait) * 1000);
  const shot = await send("Page.captureScreenshot", { format: "png" });
  await Deno.writeFile(`${out}/screensaver-${slug}.png`, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));
  for (const line of console_lines) console.error(`  ${slug}: ${line}`);
}
ws.close();
await server.shutdown();
'

# Everything the app needs to reach a GL context in a headless container. Without
# the swiftshader flags `getContext("webgl2")` returns null and the stage gives
# up, which looks exactly like a shader that did not compile.
def chromium-args [size: string] {
    [
        "--headless=new"
        "--no-sandbox"
        "--disable-dev-shm-usage"
        "--enable-unsafe-swiftshader"
        "--use-gl=angle"
        "--use-angle=swiftshader"
        "--hide-scrollbars"
        $"--window-size=($size | str replace 'x' ',')"
        $"--remote-debugging-port=($CDP_PORT)"
        "--remote-allow-origins=*"
        "about:blank"
    ]
}

# Is there anything on this picture, or is it one flat colour?
#
# Measured over the middle of the frame, not all of it: the options button sits
# in the top-left corner and is enough on its own to make a black canvas look
# like it has something on it.
def picture-of [png: string] {
    let counts = (
        ^magick $png -gravity center -crop 60x60%+0+0 +repage
            -format "%[fx:mean] %[fx:standard_deviation]" info:
        | split row " "
    )
    { mean: ($counts.0 | into float), spread: ($counts.1 | into float) }
}

def main [
    slug?: string         # which saver to render
    --all                 # render every Shadertoy saver instead
    --wait: int = 5       # seconds to let each one run before looking
    --size: string = "800x600"
    --query: string = ""  # settings to start the saver with, as a URL query
    --console             # print the browser console, for when a shader will not compile
    --dir: string = "target/dx/homepage/release/web/public"
] {
    let chromium = (find-chromium)
    if $chromium == null {
        log-fail "no chromium in /nix/store"
        exit 2
    }
    if not ($"($dir)/index.html" | path exists) {
        log-fail $"no bundle at ($dir); run `just build-whole` first"
        exit 2
    }
    let root = ($dir | path expand)

    let slugs = if $all {
        # The tier, in the order they are registered.
        [alienbeacon batteredplanet bestill bubblecolors darktransit downfall
         driftclouds elementalring fluxcore gimbalharmonics goldenapollian
         hexplasma logarithmiccircles neongravity neontriangulator noxfire
         prococean protophore rigrekt selfreflect skyline stardome starnest
         stripeytorus synthwavecity topologica trainmandala trizm truchetzoom
         universeball
         beats blinkbox boing cityflow cubestack cubestorm cubicgrid dangerball glknots gravitywell hexstrut hextrail hypnowheel
         kaleidocycle lockward menger quasicrystal sierpinski3d splodesic voronoi]
    } else if $slug != null {
        [$slug]
    } else {
        log-fail "give a slug or --all"
        exit 2
    }
    let paths = ($slugs | each {|s| $"($query | if ($in | is-empty) { $s } else { $"($s)?($query)" })" })

    log-info $"($chromium | path basename) on ($CDP_PORT), bundle on ($PORT)"
    let browser = (job spawn { ^$chromium ...(chromium-args $size) | complete | ignore })
    mut ready = false
    for _ in 1..40 {
        sleep 250ms
        let up = (try { http get --max-time 1sec $"http://127.0.0.1:($CDP_PORT)/json/version" | is-not-empty } catch { false })
        if $up { $ready = true; break }
    }
    if not $ready {
        try { job kill $browser }
        log-fail "the browser never came up"
        exit 2
    }

    $slugs | each {|s| rm -f $"/tmp/screensaver-($s).png" } | ignore
    let run = (^deno eval $DRIVER $root $"($PORT)" $"($CDP_PORT)" $"($wait)" "/tmp" ...$paths | complete)
    try { job kill $browser }
    if $console { $run.stderr | lines | each {|l| print -e $l } | ignore }

    mut failed = []
    for s in $slugs {
        let png = $"/tmp/screensaver-($s).png"
        if not ($png | path exists) {
            log-fail $"($s): no screenshot"
            $failed = ($failed | append $s)
            continue
        }
        let stats = (picture-of $png)
        # A canvas that never got a frame is uniformly black: no mean, no spread.
        if $stats.spread < 0.005 {
            log-fail $"($s): flat picture, mean ($stats.mean) spread ($stats.spread) -> ($png)"
            $failed = ($failed | append $s)
        } else {
            log-ok $"($s): mean ($stats.mean | math round --precision 3), spread ($stats.spread | math round --precision 3) -> ($png)"
        }
    }

    if ($all and (which magick | is-not-empty)) {
        let shots = ($slugs | each {|s| $"/tmp/screensaver-($s).png" } | where {|p| $p | path exists })
        let args = ($shots | append [-tile 6x5 -geometry 200x150+2+2 /tmp/screensaver-shadertoy.png])
        # montage complains about a missing font before it has decided it does
        # not need one, and still writes the file.
        ^magick montage ...$args | complete | ignore
        log-info "montage: /tmp/screensaver-shadertoy.png"
    }

    if ($failed | is-empty) {
        log-ok $"($slugs | length) rendered"
        exit 0
    }
    log-fail $"blank: ($failed | str join ', ')"
    exit 1
}
