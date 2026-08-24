#!/usr/bin/env nu

# test-windows.nu: window management in normal mode.
#
# Runs the checker client and a foot terminal side by side and drives the
# page like a user: drag the checker by its titlebar, click to focus and
# raise, drag the foot resize handle, close the checker with its titlebar
# button, and reload the page to prove resync. Each step is asserted from
# the DOM.
#
# Exit codes: 0 all window management works · 1 something did not · 2 setup
# failed.

const HOST_PORT = 8377
const CDP_PORT = 9228
const SOCKET = "wayland-webxr-windows"
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

// Every window with its geometry and stacking, keyed by title.
const SURVEY = `(() =>
  Array.from(document.querySelectorAll(".window")).map((w) => {
    const c = w.querySelector("canvas.surface");
    const r = w.getBoundingClientRect();
    const t = w.querySelector(".titlebar").getBoundingClientRect();
    const h = w.querySelector(".resize-handle").getBoundingClientRect();
    const b = w.querySelector(".close").getBoundingClientRect();
    return {
      title: w.querySelector(".title").innerText,
      focused: w.classList.contains("focused"),
      z: Number(w.style.zIndex || 0),
      left: r.left, top: r.top,
      cw: c.width, ch: c.height,
      tx: t.left + t.width / 2, ty: t.top + t.height / 2,
      hx: h.left + h.width / 2, hy: h.top + h.height / 2,
      bx: b.left + b.width / 2, by: b.top + b.height / 2,
    };
  })
)()`;

const survey = () => read(SURVEY);
const mouse = async (type, x, y) =>
  send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
const dragBy = async (x, y, dx, dy) => {
  await mouse("mousePressed", x, y);
  for (let i = 1; i <= 6; i++) {
    await send("Input.dispatchMouseEvent", {
      type: "mouseMoved", x: x + (dx * i) / 6, y: y + (dy * i) / 6,
    });
    await pause(30);
  }
  await mouse("mouseReleased", x + dx, y + dy);
  await pause(150);
};

await send("Page.navigate", { url });

let both = null;
for (let i = 0; i < 240; i++) {
  const s = await survey();
  if (s && s.length === 2 && s.every((w) => w.cw > 0)) { both = s; break; }
  await pause(250);
}
const result = { steps: {}, console: lines };
if (!both) {
  result.steps.two_windows = false;
  console.log(JSON.stringify(result));
  ws.close();
  Deno.exit(0);
}
result.steps.two_windows = true;
await pause(700);

const byTitle = (s, t) => s.find((w) => w.title === t);
let s = await survey();
const checker0 = byTitle(s, "checker");
const foot0 = s.find((w) => w.title !== "checker");

// Drag the checker by its titlebar.
await dragBy(checker0.tx, checker0.ty, 160, 90);
s = await survey();
const checker1 = byTitle(s, "checker");
result.steps.moved = {
  dx: Math.round(checker1.left - checker0.left),
  dy: Math.round(checker1.top - checker0.top),
};

// Click the checker: focused and on top.
await mouse("mousePressed", checker1.left + 40, checker1.top + 60);
await mouse("mouseReleased", checker1.left + 40, checker1.top + 60);
await pause(200);
s = await survey();
const checker2 = byTitle(s, "checker");
const foot2 = s.find((w) => w.title !== "checker");
result.steps.focused_on_top = checker2.focused && checker2.z > foot2.z;

// Grow the foot terminal by its resize handle.
await dragBy(foot2.hx, foot2.hy, 120, 60);
await pause(900);
s = await survey();
const foot3 = s.find((w) => w.title !== "checker");
result.steps.resized = { from: foot2.cw, to: foot3.cw };

// Close the checker with the titlebar button.
const checker3 = byTitle(s, "checker");
await mouse("mousePressed", checker3.bx, checker3.by);
await mouse("mouseReleased", checker3.bx, checker3.by);
let one = false;
for (let i = 0; i < 40; i++) {
  s = await survey();
  if (s.length === 1) { one = true; break; }
  await pause(250);
}
result.steps.closed = one;

// Reload: the surviving window must come back with pixels.
await send("Page.reload");
let back = null;
for (let i = 0; i < 80; i++) {
  back = await survey();
  if (back && back.length === 1 && back[0].cw > 0) break;
  back = null;
  await pause(250);
}
result.steps.resynced = back !== null;

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

def main [--out: string = "/tmp/webxr-compositor-windows.png"]: nothing -> nothing {
    let root = ($env.FILE_PWD | path join $BUNDLE)
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    let checker_bin = ($env.FILE_PWD | path join $CHECKER_BIN)
    if not ($root | path exists) {
        log-fail $"no bundle at ($root); run `just build` first"
        exit 2
    }
    if not ($host_bin | path exists) or not ($checker_bin | path exists) {
        log-fail "missing host or checker; run `just windows` instead"
        exit 2
    }
    if (which foot | is-empty) {
        log-fail "no foot on PATH; enter the devshell"
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
    ^pkill -x foot | complete | ignore
    ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    log-info $"host on ($HOST_PORT), checker + foot on ($SOCKET), chromium on ($CDP_PORT)"

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
    let terminal = (job spawn {
        with-env { WAYLAND_DISPLAY: $SOCKET } { ^foot /bin/sh -i | complete | ignore }
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
        try { job kill $terminal }
        try { job kill $checker }
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        ^pkill -f $checker_bin | complete | ignore
        ^pkill -x foot | complete | ignore
        ^pkill -f $"remote-debugging-port=($CDP_PORT)" | complete | ignore
        log-fail "chromium never opened its debugging port"
        exit 2
    }

    let run = (
        ^deno eval $DRIVER $"($CDP_PORT)" $"http://127.0.0.1:($HOST_PORT)/" $out
        | complete
    )
    try { job kill $browser }
    try { job kill $terminal }
    try { job kill $checker }
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore
    ^pkill -f $checker_bin | complete | ignore
    ^pkill -x foot | complete | ignore
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
    if not ($steps.two_windows? | default false) {
        $failures = ($failures | append "checker and foot never showed up together")
    } else {
        let moved = ($steps.moved? | default {dx: 0, dy: 0})
        if ($moved.dx | default 0) < 120 or ($moved.dy | default 0) < 60 {
            $failures = ($failures | append $"titlebar drag moved the window by (($moved.dx | default 0)),(($moved.dy | default 0)) instead of about 160,90")
        }
        if not ($steps.focused_on_top? | default false) {
            $failures = ($failures | append "clicking the checker did not focus and raise it")
        }
        let resized = ($steps.resized? | default {from: 0, to: 0})
        if (($resized.to | default 0) - ($resized.from | default 0)) < 40 {
            $failures = ($failures | append $"resize drag grew the terminal from (($resized.from | default 0)) to (($resized.to | default 0)) px only")
        }
        if not ($steps.closed? | default false) {
            $failures = ($failures | append "the close button did not close the checker")
        }
        if not ($steps.resynced? | default false) {
            $failures = ($failures | append "reload did not resync the surviving window")
        }
    }
    let complaints = ($report.console? | default [] | where {|l| $l =~ "EXCEPTION" })
    if ($complaints | is-not-empty) {
        $failures = ($failures | append $"the page threw: ($complaints | str join '; ')")
    }

    print $"  steps     ($steps | to json --raw)"
    print $"  picture   ($out)"

    if ($failures | is-empty) {
        log-ok "drag, focus, raise, resize, close and resync all work"
    } else {
        for f in $failures { log-fail $f }
        exit 1
    }
}
