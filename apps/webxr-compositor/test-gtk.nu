#!/usr/bin/env nu

# test-gtk.nu: a real GTK4 app with working menus.
#
# Runs gnome-calculator (GTK4 + libadwaita, software rendering) on the
# compositor and drives it like a user: the window must paint, clicking its
# headerbar menubutton must open an xdg_popup that renders as an overlay
# with pixels, and clicking back in the main surface must dismiss it.
#
# Exit codes: 0 the app and its popups work · 1 they do not · 2 setup
# failed.

const HOST_PORT = 8384
const CDP_PORT = 9235
const SOCKET = "wayland-webxr-gtk"
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

def find-calculator [] {
    let on_path = (which gnome-calculator)
    if ($on_path | is-not-empty) {
        return ($on_path | get 0.path)
    }
    let built = (^nix build --no-link --print-out-paths "nixpkgs#gnome-calculator" | complete)
    if $built.exit_code == 0 {
        return ($"($built.stdout | str trim)/bin/gnome-calculator")
    }
    null
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

const geometry = `(() => {
  const c = document.querySelector(".window > .canvas-holder > canvas.surface");
  if (!c || c.width === 0) return null;
  const r = c.getBoundingClientRect();
  return { w: c.width, h: c.height, left: r.left, top: r.top };
})()`;
let geo = null;
for (let i = 0; i < 240; i++) {
  geo = await read(geometry);
  if (geo) break;
  await pause(250);
}
const result = { steps: {}, console: lines };
if (!geo) {
  console.log(JSON.stringify(result));
  ws.close();
  Deno.exit(0);
}
result.steps.mounted = { w: geo.w, h: geo.h };
await pause(1500);

const click = async (x, y) => {
  await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
};

const popupProbe = `(() => {
  const p = document.querySelector(".popup canvas.surface");
  if (!p || p.width === 0) return null;
  const ctx = p.getContext("2d");
  const d = ctx.getImageData(0, 0, p.width, p.height).data;
  let painted = 0;
  for (let i = 0; i < d.length; i += 4) {
    if (d[i + 3] > 0 && (d[i] + d[i + 1] + d[i + 2]) > 0) painted++;
  }
  const r = p.getBoundingClientRect();
  return { w: p.width, h: p.height, painted, left: r.left, top: r.top };
})()`;

// Sweep headerbar spots until a menubutton opens a popover. The surface
// carries a ~25px client-side shadow inset, so the headerbar row sits at
// about y = 42; the hamburger and the mode menubutton sit mid-row, left of
// the window buttons.
const spots = [
  [247, 42], [183, 42], [geo.w - 163, 42], [geo.w / 2 - 22, 42], [120, 42],
];
let popup = null;
let used = null;
for (const [sx, sy] of spots) {
  await click(geo.left + sx, geo.top + sy);
  for (let i = 0; i < 10; i++) {
    await pause(200);
    popup = await read(popupProbe);
    if (popup) break;
  }
  if (popup) { used = [Math.round(sx), sy]; break; }
}
result.steps.popup = popup;
result.steps.spot = used;

if (popup) {
  // The grab moves keyboard focus into the popover: ArrowDown must change
  // the highlight (popup pixels), Escape must close it.
  const sample = `(() => {
    const p = document.querySelector(".popup canvas.surface");
    if (!p || p.width === 0) return null;
    const d = p.getContext("2d").getImageData(0, 0, p.width, p.height).data;
    let sum = 0;
    for (let i = 0; i < d.length; i += 53) sum += d[i];
    return sum;
  })()`;
  const key = async (code) => {
    await send("Input.dispatchKeyEvent", { type: "keyDown", code, key: code });
    await send("Input.dispatchKeyEvent", { type: "keyUp", code, key: code });
  };
  const before = await read(sample);
  await key("ArrowDown");
  let navigated = false;
  for (let i = 0; i < 15; i++) {
    await pause(200);
    const now = await read(sample);
    if (now !== null && now !== before) { navigated = true; break; }
  }
  result.steps.navigated = navigated;

  await key("Escape");
  let escaped = false;
  for (let i = 0; i < 15; i++) {
    await pause(200);
    if (!(await read(`document.querySelector(".popup") !== null`))) { escaped = true; break; }
  }
  result.steps.escaped = escaped;
}

if (result.steps.escaped) {
  // Reopen the same menu; focus must have returned to the window for the
  // menubutton to respond at all.
  popup = null;
  await click(geo.left + used[0], geo.top + used[1]);
  for (let i = 0; i < 15; i++) {
    await pause(200);
    popup = await read(popupProbe);
    if (popup) break;
  }
  result.steps.reopened = popup !== null;
}

if (popup) {
  // A click back in the main surface must dismiss the menu. The point must
  // be inside the real viewport (headless screens can be shorter than the
  // window) and left of the popover, which is why it hugs the left edge.
  await click(geo.left + 60, geo.top + Math.min(geo.h - 60, 500));
  let gone = false;
  for (let i = 0; i < 15; i++) {
    await pause(200);
    if (!(await read(`document.querySelector(".popup") !== null`))) { gone = true; break; }
  }
  result.steps.dismissed = gone;
}

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

def main [--out: string = "/tmp/webxr-compositor-gtk.png"]: nothing -> nothing {
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
    let calc = (find-calculator)
    if $calc == null {
        log-fail "no gnome-calculator on PATH or in nixpkgs"
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
    ^pkill -f gnome-calculator | complete | ignore
    ^pkill -f gnome-calculator | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    log-info $"host on ($HOST_PORT), calculator on ($SOCKET), chromium on ($CDP_PORT)"

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

    let app = (job spawn {
        with-env {
            WAYLAND_DISPLAY: $SOCKET
            GDK_BACKEND: "wayland"
            GSK_RENDERER: "cairo"
            GTK_A11Y: "none"
            NO_AT_BRIDGE: "1"
        } { ^$calc | complete | ignore }
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
        try { job kill $app }
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f gnome-calculator | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $out
        | complete
    )
    try { job kill $browser }
    try { job kill $app }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f gnome-calculator | complete | ignore
    ^pkill -f gnome-calculator | complete | ignore
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
        $failures = ($failures | append "the calculator window never painted")
    } else {
        let popup = ($steps.popup? | default null)
        if $popup == null {
            $failures = ($failures | append "no menubutton click produced a popup overlay")
        } else {
            if ($popup.painted | default 0) < 500 {
                $failures = ($failures | append $"the popup canvas has only (($popup.painted | default 0)) painted pixels")
            }
            if not ($steps.navigated? | default false) {
                $failures = ($failures | append "ArrowDown never changed the menu highlight")
            }
            if not ($steps.escaped? | default false) {
                $failures = ($failures | append "Escape did not close the menu")
            }
            if not ($steps.reopened? | default false) {
                $failures = ($failures | append "the menu would not reopen after Escape")
            }
            if not ($steps.dismissed? | default false) {
                $failures = ($failures | append "clicking the main surface did not dismiss the popup")
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
        log-ok "a GTK4 app runs with working menus and dismissal"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
