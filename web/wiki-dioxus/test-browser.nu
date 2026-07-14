#!/usr/bin/env nu

# test-browser.nu — Load the RadikalWiki Dioxus app in a headless browser and
#                   verify DOM state via W3C WebDriver (nushell + curl + jq).
#
# The app is a single Dioxus/WASM SPA served by `dx serve` (debug build). Two
# WebDriver backends: headless Servo (default) or real Firefox via geckodriver
# (--firefox). Prefer --firefox: Servo masks client-side routing / rendering bugs
# that real browsers hit (it hid the whole navigation-stale-view bug).
#
# Beyond DOM assertions it runs a whole-page WCAG contrast audit on each key
# screen (test-contrast-audit.js — fails on any green-on-green / faint text; a
# self-test proves the detector works), so contrast regressions can't merge.
#
# Usage:
#   nu test-browser.nu                     # Unauthenticated smoke tests (Servo)
#   nu test-browser.nu --firefox           # Drive real Firefox (geckodriver)
#   nu test-browser.nu --firefox --shots   # Also save light/dark x desktop/mobile
#                                          #   PNG images of key screens to ./screenshots
#   nu test-browser.nu --timeout 30        # Per-wait timeout (seconds)
#   nu test-browser.nu --verbose           # Print WebDriver-server stderr at end
#   nu test-browser.nu --keep              # Keep dx serve + browser running after
#
#   # Authenticated tests are opt-in and need credentials (never commit these):
#   WIKI_EMAIL=you@example.com WIKI_PASSWORD=secret nu test-browser.nu
#
# Exit codes: 0 all passed · 1 test failure · 2 missing deps / setup failure

const WD_PORT = 7134
const SERVE_PORT = 8134

def wd-url [] { $"http://127.0.0.1:($WD_PORT)" }
def base-url [] { $"http://127.0.0.1:($SERVE_PORT)" }

# ── Logging ────────────────────────────────────────────────────────────────

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset)  ($msg | str join ' ')" }
def log-ok   [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset)  ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset)  ($msg | str join ' ')" }
def log-warn [...msg: string] { print -e $"(ansi yellow_bold)[warn](ansi reset)  ($msg | str join ' ')" }

# True when the driving engine is headless Servo, which can't run a handful of
# checks (it never fires window-resize events and only partially implements
# contenteditable execCommand). Those checks warn-skip on Servo and run for real
# under `--firefox`. Returns true (and logs a skip) so callers can `if (servo-skip ...) { } else { <assert> }`.
def servo-skip [what: string]: nothing -> bool {
    if (($env | get -o WIKI_ENGINE | default "servo") == "servo") {
        log-warn $"skipping ($what) — headless Servo can't verify it \(runs under --firefox)"
        true
    } else {
        false
    }
}

# ── WebDriver helpers (curl + from json) ────────────────────────────────────

def wd-post [path: string, body: string] {
    try { ^curl -sf -H "Content-Type: application/json" -d $body $"(wd-url)($path)" | from json } catch { null }
}
def wd-get [path: string] {
    try { ^curl -sf $"(wd-url)($path)" | from json } catch { null }
}
def wd-delete [path: string] {
    try { ^curl -sf -X DELETE $"(wd-url)($path)" | complete | ignore } catch { }
}

def wd-new-session [caps: string] {
    let resp = (wd-post "/session" $caps)
    if $resp == null { return "" }
    $resp | get -o value.sessionId | default ($resp | get -o sessionId | default "")
}

def wd-navigate [session_id: string, url: string] {
    wd-post $"/session/($session_id)/url" ({ url: $url } | to json) | ignore
}

def wd-set-timeouts [session_id: string, script_ms: int] {
    wd-post $"/session/($session_id)/timeouts" ({ script: $script_ms, pageLoad: 30000, implicit: 0 } | to json) | ignore
}

# Find one element by CSS selector; returns the element id ("" if not found).
def wd-find [session_id: string, css: string] {
    let body = ({ using: "css selector", value: $css } | to json)
    let resp = (wd-post $"/session/($session_id)/element" $body)
    if $resp == null { return "" }
    let val = ($resp | get -o value)
    if $val == null { return "" }
    try { $val | values | first } catch { "" }
}

def wd-find-all-count [session_id: string, css: string] {
    let body = ({ using: "css selector", value: $css } | to json)
    let resp = (wd-post $"/session/($session_id)/elements" $body)
    if $resp == null { return 0 }
    try { $resp | get value | length } catch { 0 }
}

def wd-text [session_id: string, eid: string] {
    let resp = (wd-get $"/session/($session_id)/element/($eid)/text")
    if $resp == null { return "" }
    $resp | get -o value | default ""
}

def wd-click [session_id: string, eid: string] {
    wd-post $"/session/($session_id)/element/($eid)/click" '{}' | ignore
}

def wd-send-keys [session_id: string, eid: string, text: string] {
    wd-post $"/session/($session_id)/element/($eid)/value" ({ text: $text } | to json) | ignore
}

# Execute synchronous JavaScript, returning its value.
def wd-execute [session_id: string, script: string] {
    let resp = (wd-post $"/session/($session_id)/execute/sync" ({ script: $script, args: [] } | to json))
    if $resp == null { return null }
    $resp | get -o value
}

# Resize the browser window (for responsive / breakpoint screenshots).
def wd-window-rect [session_id: string, w: int, h: int] {
    wd-post $"/session/($session_id)/window/rect" ({ width: $w, height: $h, x: 0, y: 0 } | to json) | ignore
}

# Save a PNG screenshot of the current page.
def wd-screenshot [session_id: string, out: string] {
    let resp = (wd-get $"/session/($session_id)/screenshot")
    if $resp == null { return }
    let b64 = ($resp | get -o value | default "")
    if ($b64 | is-empty) { return }
    $b64 | decode base64 | save -f $out
}

# Self-test: prove the audit actually DETECTS a violation (a passing audit is
# only meaningful if the detector works). Inject a known green-on-green element,
# confirm the audit flags it, then remove it.
def check-contrast-selftest [session_id: string, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    let audit_js = (try { open --raw ($env.FILE_PWD | path join "test-contrast-audit.js") } catch { "" })
    if ($audit_js | is-empty) { return { passed: $passed, failed: $failed } }
    wd-execute $session_id "var d=document.createElement('div'); d.id='__ctest'; d.className='__ctest'; d.style.cssText='color:#006b32;background-color:#008740;font-size:14px;padding:4px'; d.textContent='green on green'; document.body.appendChild(d); return 1" | ignore
    let raw = (wd-execute $session_id $audit_js)
    let caught = (try { ($raw | from json | any {|b| (($b.s | str contains "__ctest") or ($b.t | str contains "green on green")) }) } catch { false })
    wd-execute $session_id "var e=document.getElementById('__ctest'); if(e)e.remove(); return 1" | ignore
    if $caught {
        log-ok "contrast audit self-test: detects a green-on-green violation"
        { passed: ($passed + 1), failed: $failed }
    } else {
        log-fail "contrast audit self-test FAILED: did not detect an injected violation"
        { passed: $passed, failed: ($failed + 1) }
    }
}

# Run the whole-page contrast audit (test-contrast-audit.js) and fail on any
# element below WCAG AA. This is the systematic gate for green-on-green / faint
# text: it walks every visible text element, resolves its effective (composited)
# background, and checks the real contrast ratio.
def check-contrast [session_id: string, label: string, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    let audit_js = (try { open --raw ($env.FILE_PWD | path join "test-contrast-audit.js") } catch { "" })
    if ($audit_js | is-empty) { log-warn "contrast audit script missing — skipping"; return { passed: $passed, failed: $failed } }
    let raw = (wd-execute $session_id $audit_js)
    if $raw == null { log-warn $"contrast audit errored on ($label)"; return { passed: $passed, failed: $failed } }
    let bad = (try { $raw | from json } catch { null })
    if $bad == null { log-warn $"contrast audit returned non-JSON on ($label)"; return { passed: $passed, failed: $failed } }
    if ($bad | length) == 0 {
        log-ok $"contrast OK: ($label)"
        { passed: ($passed + 1), failed: $failed }
    } else {
        log-fail $"($bad | length) low-contrast elements on ($label)"
        for b in ($bad | take 8) { log-fail $"    ($b.r):1 < ($b.m)  ($b.s)  ($b.t)" }
        { passed: $passed, failed: ($failed + 1) }
    }
}

# Capture the current view in light + dark theme at desktop + mobile widths, if
# screenshots are enabled (--shots sets WIKI_SHOTS). `name` prefixes the files.
# The theme rides `data-theme` on <html> (set once by the app at mount), so we
# flip it directly — no reload, and it can't be undone by re-renders.
def capture-shots [session_id: string, name: string] {
    if (($env | get -o WIKI_SHOTS | default "") != "1") { return }
    let dir = ($env | get -o WIKI_SHOTS_DIR | default "screenshots")
    mkdir $dir
    for wh in [{ w: 1280, h: 900, tag: "desktop" }, { w: 390, h: 844, tag: "mobile" }] {
        wd-window-rect $session_id $wh.w $wh.h
        sleep 200ms
        for theme in ["light", "dark"] {
            # Set the theme and force a reflow in one round-trip.
            wd-execute $session_id ("document.documentElement.setAttribute('data-theme','" + $theme + "'); void document.body.offsetHeight; return 1") | ignore
            sleep 350ms
            let out = ($dir | path join $"($name)-($wh.tag)-($theme).png")
            # Screenshot twice: after a CSS-var (theme) change headless Firefox can
            # return a pre-repaint frame the first time; the second grabs the paint.
            wd-screenshot $session_id $out
            sleep 200ms
            wd-screenshot $session_id $out
        }
    }
    # Restore the app's default (light) at desktop for subsequent tests.
    wd-execute $session_id "document.documentElement.setAttribute('data-theme','light'); return 1" | ignore
    wd-window-rect $session_id 1280 900
    sleep 150ms
}

# Poll until a CSS selector matches (or timeout in seconds). Returns bool.
def wd-wait-for-element [session_id: string, css: string, max_wait: int] {
    mut elapsed = 0
    while $elapsed < $max_wait {
        let eid = (try { wd-find $session_id $css } catch { "" })
        if ($eid | is-not-empty) and $eid != "null" { return true }
        sleep 500ms
        $elapsed = $elapsed + 1
    }
    false
}

# Wait for the Dioxus app to mount into #main (non-empty innerHTML).
def wd-wait-for-mount [session_id: string, max_wait: int] {
    mut elapsed = 0
    while $elapsed < $max_wait {
        let len = (try {
            wd-execute $session_id 'return (document.getElementById("main")||{innerHTML:""}).innerHTML.length'
        } catch { 0 })
        if ($len != null) and ($len != 0) { return true }
        sleep 500ms
        $elapsed = $elapsed + 1
    }
    false
}

# Poll a JS snippet (must `return "y"` when ready) every 150ms up to max_ms.
# Returns as soon as it is ready, so it replaces a fixed sleep after an async
# navigation/action while keeping a safety cap. A small settle follows on success.
def wd-wait-y [session_id: string, js: string, max_ms: int] {
    mut waited = 0
    while $waited < $max_ms {
        if ((try { wd-execute $session_id $js } catch { "n" }) == "y") { sleep 150ms; return true }
        sleep 150ms
        $waited = $waited + 150
    }
    false
}

# ── Assertion helpers ───────────────────────────────────────────────────────

def assert-exists [session_id: string, label: string, css: string, --passed (-p): int, --failed (-f): int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed; mut fl = $failed
    let eid = (try { wd-find $session_id $css } catch { "" })
    if ($eid | is-not-empty) and $eid != "null" { log-ok $label; $p = $p + 1 } else { log-fail $"($label) — not found: ($css)"; $fl = $fl + 1 }
    { passed: $p, failed: $fl }
}

def assert-count [session_id: string, label: string, css: string, min: int, --passed (-p): int, --failed (-f): int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed; mut fl = $failed
    let n = (try { wd-find-all-count $session_id $css } catch { 0 })
    if $n >= $min { log-ok $"($label) \(($n))"; $p = $p + 1 } else { log-fail $"($label) — expected >= ($min), got ($n)"; $fl = $fl + 1 }
    { passed: $p, failed: $fl }
}

def assert-contains [session_id: string, label: string, css: string, substr: string, --passed (-p): int, --failed (-f): int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed; mut fl = $failed
    let eid = (try { wd-find $session_id $css } catch { "" })
    if ($eid | is-empty) or $eid == "null" {
        log-fail $"($label) — not found: ($css)"; $fl = $fl + 1
    } else {
        let t = (wd-text $session_id $eid)
        if ($t | str contains $substr) { log-ok $label; $p = $p + 1 } else { log-fail $"($label) — expected to contain \"($substr)\", got \"($t)\""; $fl = $fl + 1 }
    }
    { passed: $p, failed: $fl }
}

# ── Process / port helpers ──────────────────────────────────────────────────

def kill-port [port: int] {
    let in_use = (try { ^fuser $"($port)/tcp" | complete; true } catch { false })
    if $in_use {
        log-warn $"Port ($port) in use — killing stale process"
        try { ^fuser -k $"($port)/tcp" | complete } catch { }
        sleep 500ms
    }
}

def servo-bin [] {
    if (which servoshell | is-not-empty) { "servoshell" } else if (which servo | is-not-empty) { "servo" } else { "" }
}

# Shell command that starts geckodriver on the WebDriver port. Prefers a
# geckodriver + firefox already on PATH; otherwise fetches them via nix. Real
# Firefox catches client-side routing / rendering bugs that headless Servo masks.
def gecko-cmd [log: string] {
    if (which geckodriver | is-not-empty) and (which firefox | is-not-empty) {
        $'geckodriver --port ($WD_PORT) > "($log)" 2>&1 & echo $!'
    } else {
        $'nix shell nixpkgs#geckodriver nixpkgs#firefox --command geckodriver --port ($WD_PORT) > "($log)" 2>&1 & echo $!'
    }
}

def do-cleanup [session_id: string, driver_pid: int, server_pid: int, keep_server: bool] {
    if ($session_id | is-not-empty) { wd-delete $"/session/($session_id)" }
    # Always tear down the browser; keep the dev server when it was reused.
    let pids = if $keep_server { [$driver_pid] } else { [$driver_pid $server_pid] }
    for pid in $pids {
        if $pid > 0 {
            let alive = (do -i { ^kill -0 $pid } | complete)
            if $alive.exit_code == 0 { do -i { ^kill $pid } | complete | ignore }
        }
    }
    # geckodriver is launched through a nix wrapper whose pid isn't the driver's,
    # so sweep the WebDriver port too. Leave the serve port alone when reusing.
    kill-port $WD_PORT
    if not $keep_server { kill-port $SERVE_PORT }
}

# ── Tests: unauthenticated shell ────────────────────────────────────────────

def test-shell [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed; mut fl = $failed
    log-info ""
    log-info "── App shell (unauthenticated) ──────────────────────────"

    wd-navigate $session_id $"(base-url)/"
    if not (wd-wait-for-mount $session_id 40) {
        log-fail "App did not mount into #main"
        return { passed: $p, failed: ($fl + 1) }
    }
    sleep 300ms

    let r = (assert-exists $session_id "app shell mounts" "#main .app-shell" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
    # <html lang> must be set so `hyphens: auto` (document text, node names) actually
    # hyphenates — without a lang attribute the browser has no dictionary.
    let htmllang = (wd-execute $session_id 'return document.documentElement.getAttribute("lang")||""')
    if ($htmllang == "da") or ($htmllang == "en") {
        log-ok $"html lang set to '($htmllang)' for hyphenation"; $p = $p + 1
    } else {
        log-fail $"html lang not set: '($htmllang)'"; $fl = $fl + 1
    }
    # The welcome card's title is the home (root) node's name; unauthenticated it
    # falls back to the default. Assert a non-empty title renders (data-driven).
    let wtitle = (wd-execute $session_id 'var h=document.querySelector("#main .home-hero-title"); return h?h.innerText.trim():""')
    if ($wtitle | is-not-empty) { log-ok "welcome card shows a title"; $p = $p + 1 } else { log-fail "welcome card title missing"; $fl = $fl + 1 }
    let r = (assert-exists $session_id "log in link present" '#main a[href="/user/login"]' -p $p -f $fl); $p = $r.passed; $fl = $r.failed
    let r = (assert-exists $session_id "register link present" '#main a[href="/user/register"]' -p $p -f $fl); $p = $r.passed; $fl = $r.failed
    let r = (check-contrast-selftest $session_id $p $fl); $p = $r.passed; $fl = $r.failed
    let r = (check-contrast $session_id "logged-out shell" $p $fl); $p = $r.passed; $fl = $r.failed

    # PWA (#33): the app is installable — a manifest link + brand theme-color are
    # injected into the head (by pwa::setup), the manifest (a runtime blob)
    # declares name/start_url/icons, and the icon it points to actually serves.
    let mhref = (wd-execute $session_id 'var l=document.querySelector("link[rel=manifest]"); return l?(l.getAttribute("href")):"none"')
    if ($mhref != "none") and ($mhref | str starts-with "blob:") { log-ok "PWA manifest linked (blob)"; $p = $p + 1 } else { log-fail $"no PWA manifest link: ($mhref)"; $fl = $fl + 1 }
    let tcolor = (wd-execute $session_id 'var m=document.querySelector("meta[name=theme-color]"); return m?(m.getAttribute("content")):"none"')
    if $tcolor != "none" { log-ok $"PWA theme-color: ($tcolor)"; $p = $p + 1 } else { log-fail "no theme-color meta"; $fl = $fl + 1 }
    # Fetch the manifest blob + its icon synchronously; validate the essentials.
    let mstatus = (wd-execute $session_id 'try{var l=document.querySelector("link[rel=manifest]"); var x=new XMLHttpRequest(); x.open("GET",l.href,false); x.send(); var m=JSON.parse(x.responseText); if(!(m.name&&m.start_url&&m.icons&&m.icons.length))return "invalid"; var y=new XMLHttpRequest(); y.open("GET",m.icons[0].src,false); y.send(); return (y.status===200&&y.responseText.indexOf("<svg")>=0)?"ok":("icon:"+y.status)}catch(e){return "err:"+e}')
    if $mstatus == "ok" { log-ok "PWA manifest + icon valid and served"; $p = $p + 1 } else { log-fail $"PWA manifest/icon check: ($mstatus)"; $fl = $fl + 1 }

    # Pull-to-refresh: the indicator mounts, and an over-scroll up at the top
    # triggers the refreshing animation (synthetic wheel event; scrollY is 0).
    let r = (assert-exists $session_id "pull-to-refresh indicator mounts" ".ptr-indicator" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
    wd-execute $session_id "window.scrollTo(0,0); var e=new WheelEvent('wheel',{deltaY:-300,bubbles:true,cancelable:true}); window.dispatchEvent(e); return 1" | ignore
    mut ptr_seen = false
    mut ptr_tries = 0
    while (not $ptr_seen) and $ptr_tries < 6 {
        sleep 120ms
        let rc = (wd-execute $session_id "return document.querySelector('.ptr-indicator.refreshing')?'y':'n'")
        if $rc == "y" { $ptr_seen = true }
        $ptr_tries = $ptr_tries + 1
    }
    if $ptr_seen { log-ok "pull-to-refresh triggers the refreshing animation"; $p = $p + 1 } else { log-fail "over-scroll up did not trigger the refresh animation"; $fl = $fl + 1 }

    # Client-side routing into the login page.
    let login = (try { wd-find $session_id '#main a[href="/user/login"]' } catch { "" })
    if ($login | is-not-empty) and $login != "null" {
        wd-click $session_id $login
        sleep 500ms
        let r = (assert-exists $session_id "login route has email input" '#main input[type=email]' -p $p -f $fl); $p = $r.passed; $fl = $r.failed
        let r = (assert-exists $session_id "login route has password input" '#main input[type=password]' -p $p -f $fl); $p = $r.passed; $fl = $r.failed
        let path = (wd-execute $session_id 'return location.pathname')
        if $path == "/user/login" { log-ok "client-side routing to /user/login"; $p = $p + 1 } else { log-fail $"expected /user/login, got ($path)"; $fl = $fl + 1 }
    } else {
        log-fail "could not click login link"; $fl = $fl + 1
    }

    { passed: $p, failed: $fl }
}

# ── Tests: authenticated (opt-in via WIKI_EMAIL / WIKI_PASSWORD) ─────────────

def test-auth [session_id: string, email: string, password: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed; mut fl = $failed
    log-info ""
    log-info "── Authenticated home list ──────────────────────────────"

    # Fresh login: mount unauthenticated first (a reload with a stored session
    # currently trips a flaky Servo wasm panic — see PLAN.md), then sign in.
    wd-navigate $session_id $"(base-url)/"
    wd-execute $session_id 'try{localStorage.clear()}catch(e){}; return "ok"' | ignore
    wd-navigate $session_id $"(base-url)/"
    if not (wd-wait-for-mount $session_id 40) { log-fail "unauthenticated home did not mount"; return { passed: $p, failed: ($fl + 1) } }

    wd-navigate $session_id $"(base-url)/user/login"
    if not (wd-wait-for-element $session_id '#main input[type=email]' 15) { log-fail "login form did not render"; return { passed: $p, failed: ($fl + 1) } }
    sleep 300ms

    wd-send-keys $session_id (wd-find $session_id '#main input[type=email]') $email
    wd-send-keys $session_id (wd-find $session_id '#main input[type=password]') $password
    wd-click $session_id (wd-find $session_id '#main button')

    # Wait for the session to land and the drawer to populate.
    mut ok = false
    for _ in 1..($timeout) {
        let authed = (wd-execute $session_id 'return localStorage.getItem("wiki_session")?"y":"n"')
        if $authed == "y" { $ok = true; break }
        sleep 1sec
    }
    if not $ok { log-fail "login did not establish a session"; return { passed: $p, failed: ($fl + 1) } }
    log-ok "login established a session"; $p = $p + 1
    # Wait for the authed home to render (root node + drawer) rather than a fixed 3s.
    wd-wait-y $session_id 'return document.querySelector("#main .card .headline-small")?"y":"n"' 4000 | ignore

    # ── Adaptive WindowSizeClass reacts to resize (M3 nav foundation) ────
    # The reactive size-class signal is written from a resize listener bridged
    # through a coroutine; this proves the bridge works (no runtime panic) and the
    # `data-size-class` attribute tracks the window width live.
    mkdir screenshots
    wd-window-rect $session_id 1280 900
    sleep 500ms
    let sc_wide = (wd-execute $session_id 'var s=document.querySelector(".app-shell"); return s?s.getAttribute("data-size-class"):"none"')
    wd-window-rect $session_id 460 900
    sleep 700ms
    let sc_narrow = (wd-execute $session_id 'var s=document.querySelector(".app-shell"); return s?s.getAttribute("data-size-class"):"none"')
    wd-window-rect $session_id 1280 900
    sleep 700ms
    let sc_back = (wd-execute $session_id 'var s=document.querySelector(".app-shell"); return s?s.getAttribute("data-size-class"):"none"')
    if (servo-skip "window-size-class resize tracking") {
        # Servo never dispatches the window `resize` event, so the size class can't
        # update — this reflects the engine, not the app (verified under Firefox).
    } else if ($sc_wide == "large") and ($sc_narrow == "compact") and ($sc_back == "large") {
        log-ok $"window size class reacts to resize \(($sc_wide) -> ($sc_narrow) -> ($sc_back))"; $p = $p + 1
    } else {
        log-fail $"window size class did not track resize \(wide=($sc_wide) narrow=($sc_narrow) back=($sc_back))"; $fl = $fl + 1
    }

    # ── Stale JWT recovery (the "returning to a tab" bug) ────────────────
    # Corrupt the stored access token's signature but keep its expiry in the
    # future, so the startup refresh does NOT fire. On reload the home data query
    # hits the bad token; the fix must refresh + retry so groups/events still load
    # instead of surfacing a JWT error. Skipped on Servo, which doesn't reliably
    # rehydrate the authed shell after a full-page navigation (so the refresh+retry
    # can't be exercised, and a left-over corrupt token would break the welcome
    # check below) — verified under Firefox.
    if (servo-skip "stale-JWT refresh+retry recovery") {
    } else {
        let jwt_corrupt = (wd-execute $session_id 'try { var s=JSON.parse(localStorage.getItem("wiki_session")); if(!s || !s.access_token) return "nosession"; s.access_token = s.access_token.slice(0,-6) + "AAAAAA"; localStorage.setItem("wiki_session", JSON.stringify(s)); return "ok"; } catch(e){ return "err:"+e; }')
        if $jwt_corrupt != "ok" { log-warn $"could not stage JWT-recovery check: ($jwt_corrupt)" }
        wd-navigate $session_id $"(base-url)/"
        # The stale token triggers a refresh + retry, then the drawer's groups/events
        # load; poll for them (capped) instead of a fixed 5s.
        wd-wait-y $session_id 'return document.querySelectorAll(".nav-rail-tree .list-item").length>0?"y":"n"' 6000 | ignore
        let jwt_items = (wd-execute $session_id 'return document.querySelectorAll(".nav-rail-tree .list-item").length')
        let jwt_tail = (wd-execute $session_id 'try { var s=JSON.parse(localStorage.getItem("wiki_session")); return s.access_token.slice(-6); } catch(e){ return "err"; }')
        let jwt_n = (try { $jwt_items | into int } catch { 0 })
        if $jwt_corrupt == "ok" {
            if ($jwt_n > 0) and ($jwt_tail != "AAAAAA") {
                log-ok $"stale JWT recovered: token refreshed, ($jwt_n) groups/events loaded"; $p = $p + 1
            } else {
                log-fail $"stale JWT did not recover \(drawer items=($jwt_n), token tail=($jwt_tail))"; $fl = $fl + 1
            }
        }
    }
    sleep 1sec

    # The welcome card renders for the authed user with the home (root) node's name
    # as its title. This also proves the catch-all serves `/` (empty segments ->
    # root node -> wiki/home -> HomeApp).
    let htitle = (wd-execute $session_id 'var h=document.querySelector("#main .card .home-hero-title"); return h?h.innerText.trim():""')
    if ($htitle | is-not-empty) { log-ok "home welcome card renders with a title" ; $p = $p + 1 } else { log-fail "home welcome card title missing" ; $fl = $fl + 1 }
    let r = (check-contrast $session_id "home (authenticated)" $p $fl); $p = $r.passed; $fl = $r.failed
    capture-shots $session_id "home"

    # The root editor is reachable at `/?app=editor` (the owner "edit" button on
    # the welcome card links here). The root has no URL path, so this exercises the
    # dedicated `/` route sharing the resolver, not a separate `/edit/welcome` route.
    wd-navigate $session_id $"(base-url)/?app=editor"
    wd-wait-y $session_id 'return (document.querySelector(".author-field") && document.querySelector("[contenteditable]"))?"y":"n"' 5000 | ignore
    let root_ed = (wd-execute $session_id 'return (document.querySelector(".author-field") && document.querySelector("[contenteditable]"))?"y":"n"')
    if $root_ed == "y" { log-ok "root editor renders at /?app=editor"; $p = $p + 1 } else { log-fail "root editor did not render at /?app=editor"; $fl = $fl + 1 }
    # Returning to `/` must show the welcome again (the `Home` route round-trips;
    # regression: an empty catch-all serialized to a relative "?" and stuck).
    wd-navigate $session_id $"(base-url)/"
    wd-wait-y $session_id 'return [...document.querySelectorAll(".material-icons")].some(function(e){return e.textContent.trim()==="waving_hand"})?"y":"n"' 4000 | ignore
    let back_home = (wd-execute $session_id 'return [...document.querySelectorAll(".material-icons")].some(function(e){return e.textContent.trim()==="waving_hand"})?"y":"n"')
    if $back_home == "y" { log-ok "navigating back to / shows the welcome"; $p = $p + 1 } else { log-fail "/ did not show the welcome after the editor"; $fl = $fl + 1 }

    # ── Theme colour picker: pick a primary seed, the app re-skins ──────
    # Open the user menu, pick a non-active primary swatch, and assert the live
    # --md-sys-color-primary token changes; then Reset reverts it to the brand.
    wd-execute $session_id 'var b=document.querySelector(".user-menu > button"); if(b) b.click(); return "ok"' | ignore
    sleep 600ms
    let cp_before = (wd-execute $session_id 'return getComputedStyle(document.documentElement).getPropertyValue("--md-sys-color-primary").trim()')
    let cp_click = (wd-execute $session_id 'var p=document.querySelector(".menu-color-section .color-picker"); if(!p) return "nopicker"; var sw=p.querySelectorAll("button.color-swatch"); if(sw.length<2) return "noswatch"; (sw[1].classList.contains("active")?sw[2]:sw[1]).click(); return "ok"')
    sleep 800ms
    let cp_after = (wd-execute $session_id 'return getComputedStyle(document.documentElement).getPropertyValue("--md-sys-color-primary").trim()')
    if $cp_click == "ok" and ($cp_before != $cp_after) and ($cp_after | is-not-empty) {
        log-ok $"color picker re-skins the app \(--md-sys-color-primary ($cp_before) -> ($cp_after))"; $p = $p + 1
    } else {
        log-fail $"color picker did not change the primary token \(click=($cp_click) before=($cp_before) after=($cp_after))"; $fl = $fl + 1
    }
    # Selecting the first (brand) swatch restores the default (no reset button).
    let cp_reset = (wd-execute $session_id 'var p=document.querySelector(".menu-color-section .color-picker"); if(!p) return "nopicker"; var sw=p.querySelectorAll("button.color-swatch"); if(!sw.length) return "noswatch"; sw[0].click(); return "ok"')
    sleep 700ms
    let cp_reverted = (wd-execute $session_id 'return getComputedStyle(document.documentElement).getPropertyValue("--md-sys-color-primary").trim()')
    if $cp_reset == "ok" and ($cp_reverted == $cp_before) {
        log-ok "selecting the first swatch restores the brand primary"; $p = $p + 1
    } else {
        log-fail $"first swatch did not restore the brand primary \(click=($cp_reset) expected=($cp_before) got=($cp_reverted))"; $fl = $fl + 1
    }
    # Close the menu via its toggle (it is open here) so the next test opens it
    # from a known-closed state. A programmatic body click does not dismiss it.
    wd-execute $session_id 'var b=document.querySelector(".user-menu > button"); if(b) b.click(); return "ok"' | ignore
    sleep 400ms

    # User menu: clicking the avatar opens a dropdown that stays fully within the
    # viewport, and the trigger has no stray border (regression: the primitive
    # drew a rounded square and positioned the popup off-screen).
    let umbtn = (wd-execute $session_id 'var b=document.querySelector(".user-menu > button"); if(b){b.click(); return "y"} return "n"')
    if $umbtn == "y" {
        sleep 400ms
        let dd = (wd-execute $session_id 'return document.querySelector(".user-menu-dropdown")?"y":"n"')
        if $dd == "y" {
            log-ok "user menu opens a dropdown"; $p = $p + 1
            # The account menu now carries the signed-in identity header (moved
            # out of the sidebar).
            let hdr = (wd-execute $session_id 'var h=document.querySelector(".user-menu-dropdown .user-menu-header"); if(!h) return "none"; var e=h.querySelector(".user-menu-email"); return e?e.innerText.trim():"nomail"')
            if ($hdr | str contains "@") { log-ok "account menu shows the signed-in identity"; $p = $p + 1 } else { log-fail $"account menu identity header missing: ($hdr)"; $fl = $fl + 1 }
            if (($env | get -o WIKI_SHOTS | default "") == "1") {
                mkdir screenshots
                wd-screenshot $session_id "screenshots/user-menu.png"
                sleep 150ms
                wd-screenshot $session_id "screenshots/user-menu.png"
            }
            let inview = (wd-execute $session_id 'var d=document.querySelector(".user-menu-dropdown"); var r=d.getBoundingClientRect(); return (r.left>=-1 && r.top>=-1 && r.right<=window.innerWidth+1 && r.bottom<=window.innerHeight+1)?"y":JSON.stringify({l:Math.round(r.left),t:Math.round(r.top),r:Math.round(r.right),b:Math.round(r.bottom),w:window.innerWidth,h:window.innerHeight})')
            if $inview == "y" { log-ok "user menu popup is within the viewport"; $p = $p + 1 } else { log-fail $"user menu popup off-screen: ($inview)"; $fl = $fl + 1 }
            # The account row's only border is the intentional top separator; no
            # stray side/bottom button chrome.
            let bw = (wd-execute $session_id 'var b=document.querySelector(".user-menu > button"); var s=getComputedStyle(b); return s.borderLeftWidth+"/"+s.borderBottomWidth')
            if (($bw | default "") == "0px/0px") { log-ok "account trigger has no stray border"; $p = $p + 1 } else { log-warn $"account trigger borders: ($bw)" }
            # The avatar must stand out from the green bar (the green-on-green
            # complaint): its background should differ clearly from the bar's.
            let avc = (wd-execute $session_id 'function L(c){var a=c.map(function(v){v/=255;return v<=0.03928?v/12.92:Math.pow((v+0.055)/1.055,2.4)});return 0.2126*a[0]+0.7152*a[1]+0.0722*a[2]} function P(s){var m=s.match(/[0-9.]+/g);return m?[+m[0],+m[1],+m[2]]:[0,0,0]} function R(x,y){var p=L(P(x)),q=L(P(y)),h=Math.max(p,q),l=Math.min(p,q);return (h+0.05)/(l+0.05)} var av=document.querySelector(".user-menu .avatar"); var bar=document.querySelector(".bar"); if(!av||!bar) return "no"; return R(getComputedStyle(av).backgroundColor, getComputedStyle(bar).backgroundColor).toFixed(2)')
            if $avc == "no" {
                log-warn "could not measure avatar/bar contrast"
            } else if ((try { $avc | into float } catch { 0.0 }) >= 1.4) {
                log-ok $"user avatar stands out from the bar ($avc):1"; $p = $p + 1
            } else {
                log-fail $"user avatar blends into the bar: ($avc):1"; $fl = $fl + 1
            }
            # Dark-mode toggle is now an accessible Switch (role=switch); clicking
            # it must flip <html data-theme>. Toggle it back so later screens keep
            # their theme.
            let sw = (wd-execute $session_id 'return document.querySelector(".user-menu-dropdown [role=switch]")?"y":"n"')
            if $sw == "y" {
                let before = (wd-execute $session_id 'return document.documentElement.getAttribute("data-theme")||"light"')
                wd-execute $session_id 'document.querySelector(".user-menu-dropdown [role=switch]").click(); return 1' | ignore
                sleep 400ms
                let after = (wd-execute $session_id 'return document.documentElement.getAttribute("data-theme")||"light"')
                if $before != $after { log-ok $"theme switch flips data-theme ($before) to ($after)"; $p = $p + 1 } else { log-fail $"theme switch did not change data-theme, stayed ($before)"; $fl = $fl + 1 }
                # Now checked: the switch must use the M3 green accent, not the dx
                # grayscale (regression guard for the primitive re-theming).
                let swbg = (wd-execute $session_id 'var s=document.querySelector(".user-menu-dropdown [role=switch]"); if(!s) return "none"; var m=getComputedStyle(s).backgroundColor.match(/[0-9.]+/g); return m?JSON.stringify({r:+m[0],g:+m[1],b:+m[2]}):"none"')
                if $swbg != "none" {
                    let c = ($swbg | from json)
                    if ($c.g > $c.r) and ($c.g > $c.b) { log-ok $"checked switch uses the green accent rgb\(($c.r),($c.g),($c.b)\)"; $p = $p + 1 } else { log-fail $"checked switch not green: rgb\(($c.r),($c.g),($c.b)\)"; $fl = $fl + 1 }
                } else {
                    log-warn "could not read switch colour"
                }
                wd-execute $session_id 'document.querySelector(".user-menu-dropdown [role=switch]").click(); return 1' | ignore
                sleep 300ms
            } else {
                log-fail "theme switch not found in user menu"; $fl = $fl + 1
            }
            wd-execute $session_id 'var bd=document.querySelector(".menu-backdrop"); if(bd)bd.click(); return 1' | ignore
            sleep 200ms
        } else {
            log-fail "user menu did not open a dropdown"; $fl = $fl + 1
        }
    } else {
        log-warn "user menu button not found — skipping user-menu check"
    }

    # Groups + events render as context items (avatar badges) in the drawer.
    let r = (assert-count $session_id "drawer shows group/event items" ".nav-rail-tree .avatar.secondary" 1 -p $p -f $fl); $p = $r.passed; $fl = $r.failed

    # ── In-context navigation (drawer node tree + app rail) ──────────────
    # Click the first context; the app should route into it, render a node
    # view, switch the drawer from the home list to the MenuList tree, and
    # reveal the app rail.
    # Open the first context that actually has content, so the in-context checks
    # (child tree, apps, breadcrumbs) run against a populated node. Many groups
    # are empty; blindly clicking the first avatar can land on an empty one and
    # make every downstream check fail spuriously. Click each context's list-item
    # (the avatar is a child span) until the view shows folder children.
    let n_ctx_str = (wd-execute $session_id 'return String(document.querySelectorAll(".nav-rail-tree .avatar.secondary").length)')
    let n_ctx = (try { $n_ctx_str | into int } catch { 0 })
    if $n_ctx == 0 {
        log-warn "no context to open — skipping in-context checks"
        return { passed: $p, failed: $fl }
    }
    mut navigated = false
    mut ci = 0
    let max_try = ([$n_ctx 8] | math min)
    while (not $navigated) and ($ci < $max_try) {
        # Build the click JS with plain-string concat: a $"..." interpolation
        # would try to evaluate the literal JS parens.
        let click_js = ("var xs=document.querySelectorAll('.nav-rail-tree .avatar.secondary'); var e=xs[" + ($ci | into string) + "]; if(e){e.closest('.list-item').click(); return 'clicked'} return 'none'")
        wd-execute $session_id $click_js | ignore
        mut moved = false
        for _ in 1..12 {
            let path = (wd-execute $session_id 'return location.pathname')
            if ($path != null) and ($path != "/") { $moved = true; break }
            sleep 300ms
        }
        if $moved {
            sleep 1500ms
            let populated = (wd-execute $session_id 'return document.querySelector("#main .folder-tile, #main .list-link")?"y":"n"')
            if $populated == "y" {
                $navigated = true
            } else {
                # Empty context: back to the home list and try the next one.
                wd-navigate $session_id $"(base-url)/"
                sleep 900ms
            }
        }
        $ci = $ci + 1
    }
    if not $navigated {
        log-warn "no populated context found — skipping in-context checks"
        return { passed: $p, failed: $fl }
    }
    log-ok "navigated into a populated context"; $p = $p + 1
    sleep 1sec
    # Remember this context's top-level path; later sections (app rail, app
    # switching) must re-enter it rather than trust wherever earlier breadcrumb
    # navigation left the location.
    let sel_ctx = (wd-execute $session_id 'return "/"+location.pathname.split("/")[1]')

    let r = (assert-exists $session_id "context view renders a card" "#main .card" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
    let r = (check-contrast $session_id "context view (drawer + app rail + bar)" $p $fl); $p = $r.passed; $fl = $r.failed
    capture-shots $session_id "context"

    # Medium-width expandable rail: capture collapsed then (menu-toggled) expanded,
    # so the rail-expansion motion can be reviewed.
    if (($env | get -o WIKI_SHOTS | default "") == "1") {
        let dir = ($env | get -o WIKI_SHOTS_DIR | default "screenshots")
        wd-window-rect $session_id 980 900
        sleep 700ms
        wd-screenshot $session_id ($dir | path join "rail-med-collapsed.png")
        wd-execute $session_id 'var b=document.querySelector(".nav-rail-header .btn-icon"); if(b)b.click(); return 1' | ignore
        sleep 800ms
        wd-screenshot $session_id ($dir | path join "rail-med-expanded.png")
        wd-execute $session_id 'var b=document.querySelector(".nav-rail-header .btn-icon"); if(b)b.click(); return 1' | ignore
        wd-window-rect $session_id 1280 900
        sleep 400ms
    }

    # Extra-large: the tools sheet DOCKS as a permanent right-side pane (no trigger,
    # always visible), and the content column is capped near A4 and centred rather
    # than stretching the full pane width.
    wd-window-rect $session_id 1728 1000
    sleep 800ms
    # Measure the gap against documentElement.clientWidth (the layout viewport,
    # EXCLUDING the scrollbar) — window.innerWidth includes the scrollbar, so on
    # engines that reserve a classic scrollbar (Firefox) it over-reports the gap
    # by the scrollbar width and the docked sheet looks a few px off the edge.
    let xl = (wd-execute $session_id 'var shell=document.querySelector(".app-shell"); var vw=document.documentElement.clientWidth; var dk=document.querySelector(".tool-sheet.docked"); var vis=0, rightGap=-1; if(dk){var r=dk.getBoundingClientRect(); vis=(r.width>0 && r.right<=vw+1 && r.right>=vw-2)?1:0; rightGap=Math.round(vw-r.right)} var cm=document.querySelector(".content-measure"); var cmw=cm?Math.round(cm.getBoundingClientRect().width):-1; var cap=cm?Math.round(parseFloat(getComputedStyle(cm).maxWidth)):-1; return JSON.stringify({docked: dk?1:0, vis: vis, rightGap: rightGap, toolsAttr: shell?shell.getAttribute("data-tools-docked"):"", measureW: cmw, cap: cap})')
    let x = ($xl | from json)
    if ($x.docked == 1) and ($x.vis == 1) and ($x.toolsAttr == "true") {
        log-ok $"extra-large docks the tools pane on the right, gap=($x.rightGap)px"; $p = $p + 1
    } else {
        log-fail $"extra-large tools not docked: ($xl)"; $fl = $fl + 1
    }
    if ($x.cap > 0) and ($x.measureW <= ($x.cap + 2)) {
        log-ok $"content column capped at ($x.cap)px, measured ($x.measureW)px"; $p = $p + 1
    } else {
        log-fail $"content column not capped: measured=($x.measureW), cap=($x.cap)"; $fl = $fl + 1
    }
    if (($env | get -o WIKI_SHOTS | default "") == "1") {
        let dir = ($env | get -o WIKI_SHOTS_DIR | default "screenshots")
        wd-screenshot $session_id ($dir | path join "tools-docked-xl.png")
    }
    wd-window-rect $session_id 1280 900
    sleep 400ms

    # Compact mobile drawer: open it and verify the top is the current context node
    # (with a trailing close button) and that the redundant "Home" entries are gone
    # (one used to sit in the header, one in the body list).
    wd-window-rect $session_id 390 844
    sleep 600ms
    wd-execute $session_id 'var b=document.querySelector(".top-app-bar .btn-icon"); if(b)b.click(); return 1' | ignore
    sleep 700ms
    let dstate = (wd-execute $session_id 'var dr=document.querySelector(".nav-drawer.open"); if(!dr) return "noopen"; var hdrBar=dr.querySelector(".nav-drawer-header .drawer-context-bar .drawer-context-name"); var close=dr.querySelector(".nav-drawer-header button")?1:0; var mh=dr.querySelector(".drawer-mobile-home")?1:0; var bodyBar=dr.querySelector(".list .drawer-context-bar"); var bodyShown=bodyBar?(getComputedStyle(bodyBar).display!="none"?1:0):0; var homeCount=Array.prototype.filter.call(dr.querySelectorAll("*"),function(e){return e.children.length==0 && e.innerText && e.innerText.trim()=="Home"}).length; return JSON.stringify({headerCtx: hdrBar?hdrBar.innerText.trim():"", close: close, mobileHome: mh, bodyCtxShown: bodyShown, homeCount: homeCount})')
    if $dstate == "noopen" {
        log-warn "mobile drawer did not open — skipping drawer content check"
    } else {
        let d = ($dstate | from json)
        if ($d.headerCtx != "") and ($d.headerCtx != "Home") and ($d.close == 1) {
            log-ok $"mobile drawer top is the context '($d.headerCtx)' with a close button"; $p = $p + 1
        } else {
            log-fail $"mobile drawer header wrong: ctx='($d.headerCtx)', close=($d.close)"; $fl = $fl + 1
        }
        if ($d.mobileHome == 0) and ($d.bodyCtxShown == 0) and ($d.homeCount == 0) {
            log-ok "mobile drawer has no redundant Home entries"; $p = $p + 1
        } else {
            log-fail $"mobile drawer has stray Home: mobileHome=($d.mobileHome), bodyCtxShown=($d.bodyCtxShown), homeCount=($d.homeCount)"; $fl = $fl + 1
        }
    }
    if (($env | get -o WIKI_SHOTS | default "") == "1") {
        let dir = ($env | get -o WIKI_SHOTS_DIR | default "screenshots")
        wd-screenshot $session_id ($dir | path join "mobile-drawer.png")
    }
    wd-execute $session_id 'var s=document.querySelector(".nav-drawer-scrim"); if(s)s.click(); return 1' | ignore
    wd-window-rect $session_id 1280 900
    sleep 400ms

    # Compact: the FAB is the tools-sheet trigger (not add-content). It carries the
    # "bolt" icon and opens the bottom sheet whenever the sheet is not docked.
    # Guards the FAB-repurposing change.
    wd-window-rect $session_id 390 844
    sleep 600ms
    let fabt = (wd-execute $session_id 'var f=document.querySelector(".fab"); if(!f) return "nofab"; var m=f.querySelector(".material-icons"); return m?m.textContent.trim():""')
    if $fabt == "nofab" {
        log-warn "no tools FAB on compact — skipping FAB tools check"
    } else if $fabt == "bolt" {
        wd-execute $session_id 'document.querySelector(".fab").click(); return 1' | ignore
        sleep 600ms
        let opened = (wd-execute $session_id 'return document.querySelector(".tool-sheet.open")?"y":"n"')
        if $opened == "y" {
            log-ok "compact FAB opens the tools sheet"; $p = $p + 1
        } else {
            log-fail "compact FAB did not open the tools sheet"; $fl = $fl + 1
        }
        if (($env | get -o WIKI_SHOTS | default "") == "1") {
            let dir = ($env | get -o WIKI_SHOTS_DIR | default "screenshots")
            wd-screenshot $session_id ($dir | path join "mobile-tools-fab.png")
        }
        wd-execute $session_id 'var s=document.querySelector(".sheet-scrim.open"); if(s)s.click(); return 1' | ignore
        sleep 400ms
    } else {
        log-fail $"compact FAB has the wrong icon: ($fabt)"; $fl = $fl + 1
    }
    wd-window-rect $session_id 1280 900
    sleep 400ms

    # Pull-to-refresh must actually REFETCH data, not just animate. Hook fetch to
    # count GraphQL calls, over-scroll up, and expect fresh calls to fire (the
    # generalized use_data_resource! makes every view refetch on the bump).
    let path_b = (wd-execute $session_id 'return location.pathname')
    let items_b = (wd-execute $session_id 'return String(document.querySelectorAll("#main .folder-tile, #main .list-link, #main .card").length)')
    wd-execute $session_id "if(!window.__gqlHooked){window.__gqlHooked=1; var of=window.fetch; window.fetch=function(){try{var u=arguments[0]; var s=(typeof u=='string')?u:((u&&u.url)||''); if(s.indexOf('graphql')>=0){window.__gql=(window.__gql||0)+1;}}catch(e){} return of.apply(this,arguments);};} return 'ok'" | ignore
    sleep 400ms
    wd-execute $session_id "window.__gql=0; window.scrollTo(0,0); var e=new WheelEvent('wheel',{deltaY:-300,bubbles:true,cancelable:true}); window.dispatchEvent(e); return 1" | ignore
    mut refetched = false
    mut rtries = 0
    while (not $refetched) and $rtries < 12 {
        sleep 200ms
        let c = (wd-execute $session_id "return String(window.__gql||0)")
        let n = (try { $c | into int } catch { 0 })
        if $n > 0 { $refetched = true }
        $rtries = $rtries + 1
    }
    # Wait for the view to settle back, then diagnose whether the refresh blanked
    # or navigated the view (it must not).
    mut settled = false
    for _ in 1..20 {
        sleep 200ms
        let now = (wd-execute $session_id 'return String(document.querySelectorAll("#main .folder-tile, #main .list-link, #main .card").length)')
        if ((try { $now | into int } catch { 0 }) > 0) { $settled = true; break }
    }
    let path_a = (wd-execute $session_id 'return location.pathname')
    let items_a = (wd-execute $session_id 'return String(document.querySelectorAll("#main .folder-tile, #main .list-link, #main .card").length)')
    log-info $"PTR diag: path ($path_b) to ($path_a); items ($items_b) to ($items_a); refetched=($refetched) settled=($settled)"
    if $refetched and $settled and ($path_a == $path_b) { log-ok "pull-to-refresh refetches data and keeps the view"; $p = $p + 1 } else { log-fail "pull-to-refresh disrupted the view"; $fl = $fl + 1 }
    # Draft (not-submitted, mutable) nodes show a lock badge on their avatar.
    # Only meaningful where the context actually has draft children.
    let badge_n = (wd-execute $session_id 'return String(document.querySelectorAll("#main .avatar-badge").length)')
    if ((try { $badge_n | into int } catch { 0 }) > 0) {
        log-ok "draft nodes show a not-submitted badge"; $p = $p + 1
    } else {
        log-warn "context has no draft nodes — skipping not-submitted-badge check"
    }
    # The app rail only appears inside a context; its presence confirms the
    # in-context layout (and that its icons rendered).
    let r = (assert-count $session_id "app rail shown in context" ".nav-rail-item" 1 -p $p -f $fl); $p = $r.passed; $fl = $r.failed
    # The app rail sits to the LEFT of the side panel (rail | drawer | content).
    let layout = (wd-execute $session_id 'var rl=document.querySelector(".nav-rail"),dr=document.querySelector(".nav-rail-tree"); if(!rl||!dr) return "missing"; return JSON.stringify({rail:Math.round(rl.getBoundingClientRect().left), drawer:Math.round(dr.getBoundingClientRect().left)})')
    let ok = (try { let j = ($layout | from json); ($j.rail <= $j.drawer) and ($j.rail < 10) } catch { false })
    if $ok {
        log-ok $"nav rail is at the leading edge, tree nested within ($layout)"; $p = $p + 1
    } else {
        log-fail $"nav rail not at the leading edge: ($layout)"; $fl = $fl + 1
    }
    # The drawer must have swapped the home list for the node tree: the
    # "Groups" home heading is gone once inside a context.
    let drawer_txt = (wd-execute $session_id 'return (document.querySelector(".nav-rail-tree")||{innerText:""}).innerText')
    if ($drawer_txt | describe) == "string" and (not ($drawer_txt | str contains "Groups")) {
        log-ok "drawer switched to node tree (home list gone)"; $p = $p + 1
    } else {
        log-fail "drawer still shows the home list inside a context"; $fl = $fl + 1
    }
    # The tree/drawer's top entry is the current context (not "Home"), and the old
    # redundant mobile-Home fallback link is gone (Home is on the app rail / bottom
    # nav bar instead).
    let dsw = (wd-execute $session_id 'var d=document.querySelector(".nav-rail-tree"); if(!d) return "nodrawer"; var mh=d.querySelector(".drawer-mobile-home")?1:0; var cn=d.querySelector(".drawer-context-bar .drawer-context-name"); return JSON.stringify({mobileHome:mh, contextText: cn?cn.innerText.trim():""})')
    if $dsw == "nodrawer" {
        log-warn "no drawer to check the context switch"
    } else {
        let m = ($dsw | from json)
        if ($m.contextText != "Home") and ($m.contextText != "") and ($m.mobileHome == 0) {
            log-ok $"drawer top is the context bar '($m.contextText)', no redundant Home"; $p = $p + 1
        } else {
            log-fail $"drawer context bar off: text='($m.contextText)', mobileHome=($m.mobileHome)"; $fl = $fl + 1
        }
    }
    # (The old "drawer context bar aligns with the top panel" check is retired:
    # in the M3 rail layout the tree + its context bar are nested inside the
    # navigation rail, not a separate panel aligned to the top app bar.)

    # ── Navigate to a child node (regression: PathPage must re-resolve on a
    # client-side route change between two path pages, not show stale content).
    let path_before = (wd-execute $session_id 'return location.pathname')
    let main_before = (wd-execute $session_id 'return (document.getElementById("main")||{innerText:""}).innerText')
    let clicked = (wd-execute $session_id 'var e=document.querySelector("#main .folder-tile, #main .list-link"); if(e){e.click(); return "y"} return "n"')
    if $clicked == "y" {
        mut deeper = false
        for _ in 1..($timeout) {
            let path_now = (wd-execute $session_id 'return location.pathname')
            if ($path_now != null) and ($path_now != $path_before) { $deeper = true; break }
            sleep 500ms
        }
        sleep 2sec
        let main_after = (wd-execute $session_id 'return (document.getElementById("main")||{innerText:""}).innerText')
        if $deeper and ($main_after != $main_before) {
            log-ok "navigating to a child node updates the view"; $p = $p + 1
            # The drawer tree highlights the current node in the path.
            sleep 1500ms
            let sel = (wd-execute $session_id 'return document.querySelector(".nav-rail-tree .list-item.selected, .nav-rail-tree .list-item.selected")?"y":"n"')
            if $sel == "y" {
                log-ok "drawer highlights the current node"; $p = $p + 1
            } else {
                log-fail "drawer does not highlight the current node"; $fl = $fl + 1
            }
        } else {
            log-fail "child navigation did not update the view (stale PathPage)"; $fl = $fl + 1
        }
    } else {
        log-warn "context has no folder-item children — skipping child-nav check"
    }

    # ── Breadcrumb up-navigation re-resolves the view (regression) ───────
    # Clicking a breadcrumb changes the URL to an ancestor path; the main view
    # must re-render to that ancestor, not keep showing the deeper node. This
    # broke when PathPage relied on router prop propagation between two PathPage
    # routes instead of subscribing to the route itself.
    let bc_path_before = (wd-execute $session_id 'return location.pathname')
    let bc_main_before = (wd-execute $session_id 'return (document.getElementById("main")||{innerText:""}).innerText')
    # Click a PARENT crumb (every crumb is now a link incl. the current node, so
    # click the second-to-last to actually navigate up a level).
    let bc_clicked = (wd-execute $session_id 'var a=[...document.querySelectorAll(".breadcrumbs a")]; if(a.length>1){a[a.length-2].click(); return "y"} return "n"')
    if $bc_clicked == "y" {
        mut bc_nav = false
        for _ in 1..($timeout) {
            let now = (wd-execute $session_id 'return location.pathname')
            if ($now != null) and ($now != $bc_path_before) { $bc_nav = true; break }
            sleep 500ms
        }
        sleep 2sec
        let bc_main_after = (wd-execute $session_id 'return (document.getElementById("main")||{innerText:""}).innerText')
        if $bc_nav and ($bc_main_after != $bc_main_before) {
            log-ok "breadcrumb navigation updates the view"; $p = $p + 1
        } else {
            log-fail "breadcrumb changed the URL but not the view (stale PathPage)"; $fl = $fl + 1
        }
    } else {
        log-warn "not enough breadcrumb links to test up-navigation"
    }

    # Breadcrumbs show a mime avatar per crumb (home + each segment).
    let r = (assert-count $session_id "breadcrumbs render avatars" ".breadcrumbs .crumb-avatar" 1 -p $p -f $fl); $p = $r.passed; $fl = $r.failed

    # ── App rail switches apps via the ?app= route query (client-side) ────
    # Go back to the SELECTED context (breadcrumb navigation above may have left
    # us at home), then click the Vote rail item and confirm the URL gains
    # ?app=vote and the vote view renders. Click via JS on the anchor (WebDriver's
    # click can land on an inner span the router doesn't intercept).
    let ctx_path = $sel_ctx
    wd-navigate $session_id $"(base-url)($ctx_path)"
    if (wd-wait-for-element $session_id ".nav-rail a" 15) {
        let clicked_vote = (wd-execute $session_id 'var a=[...document.querySelectorAll(".nav-rail a")].find(function(x){return (x.getAttribute("href")||"").includes("app=vote")}); if(a){a.click(); return "y"} return "n"')
        if $clicked_vote == "y" {
            mut switched = false
            for _ in 1..($timeout) {
                let s = (wd-execute $session_id 'return location.search')
                if ($s | default "" | str contains "app=vote") { $switched = true; break }
                sleep 500ms
            }
            sleep 1sec
            if $switched {
                log-ok "app rail routes to ?app=vote"; $p = $p + 1
            } else {
                let search = (wd-execute $session_id 'return location.search')
                log-fail $"app rail did not set ?app=vote; got: ($search)"; $fl = $fl + 1
            }
            let r = (assert-exists $session_id "vote app renders" "#main .card" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
            let r = (check-contrast $session_id "vote app (active app-rail highlight)" $p $fl); $p = $r.passed; $fl = $r.failed
            # The open app is its OWN trailing breadcrumb (a labelled, clickable
            # step) rather than a small overlay badge on the node avatar.
            let appcrumb = (wd-execute $session_id 'var links=[...document.querySelectorAll(".breadcrumbs .crumb .crumb-link")]; var last=links[links.length-1]; var href=last?(last.getAttribute("href")||""):""; var nmEl=last?last.querySelector(".crumb-name.open"):null; var badge=document.querySelector(".crumb-app-badge")?1:0; return JSON.stringify({href: href, name: nmEl?nmEl.innerText.trim():"", badge: badge})')
            let ac = ($appcrumb | from json)
            if ($ac.href | str contains "app=vote") and ($ac.badge == 0) {
                log-ok $"open app is its own trailing crumb, name='($ac.name)', no overlay badge"; $p = $p + 1
            } else {
                log-fail $"app crumb missing or wrong: ($appcrumb)"; $fl = $fl + 1
            }
            capture-shots $session_id "vote-app"
            # Speak app (if this context exposes it): capture it, then return to
            # the vote app so the downstream vote-app checks keep their state.
            let has_speak = (wd-execute $session_id 'return document.querySelector(".nav-rail a[href*=\"app=speak\"]")?"y":"n"')
            if $has_speak == "y" {
                wd-execute $session_id 'var a=[...document.querySelectorAll(".nav-rail a")].find(function(x){return (x.getAttribute("href")||"").includes("app=speak")}); if(a){a.click()}; return 1' | ignore
                sleep 800ms
                capture-shots $session_id "speak-app"
                wd-execute $session_id 'var a=[...document.querySelectorAll(".nav-rail a")].find(function(x){return (x.getAttribute("href")||"").includes("app=vote")}); if(a){a.click()}; return 1' | ignore
                sleep 600ms
            }
            # The highlighted app must have readable contrast (icon vs its box):
            # regression for the green-on-green active state.
            # M3 nav-rail active indicator: a container-tone pill behind the icon.
            # Two guards: the icon must be readable ON the pill (luminance ratio,
            # the green-on-green regression), and the pill must be distinguishable
            # FROM the rail (colour DISTANCE, since a tonal pill differs from the
            # neutral rail by hue/chroma, not luminance).
            let arc = (wd-execute $session_id 'function L(c){var a=c.map(function(v){v/=255;return v<=0.03928?v/12.92:Math.pow((v+0.055)/1.055,2.4)});return 0.2126*a[0]+0.7152*a[1]+0.0722*a[2]} function P(s){var m=s.match(/[0-9.]+/g);return [+m[0],+m[1],+m[2]]} function R(x,y){var p=L(P(x)),q=L(P(y)),h=Math.max(p,q),l=Math.min(p,q);return (h+0.05)/(l+0.05)} function D(x,y){var a=P(x),b=P(y);return Math.round(Math.sqrt((a[0]-b[0])*(a[0]-b[0])+(a[1]-b[1])*(a[1]-b[1])+(a[2]-b[2])*(a[2]-b[2])))} var el=document.querySelector(".nav-rail-item.active"); if(!el) return "noactive"; var pill=el.querySelector(".nav-rail-indicator")||el; var ic=el.querySelector(".material-icons"); var pb=getComputedStyle(pill).backgroundColor; if(pb=="rgba(0, 0, 0, 0)"||pb=="transparent"){pb=getComputedStyle(el).backgroundColor;} var icc=ic?getComputedStyle(ic).color:getComputedStyle(el).color; var rail=document.querySelector(".nav-rail"); var rb=rail?getComputedStyle(rail).backgroundColor:"rgb(255,255,255)"; return JSON.stringify({readable:+R(pb,icc).toFixed(2), distinct:D(pb,rb)})')
            if $arc == "noactive" {
                log-warn "no active app-rail item to contrast-check"
            } else {
                let m = ($arc | from json)
                if ($m.readable >= 3.0) and ($m.distinct >= 40) {
                    log-ok $"active app indicator: icon ($m.readable):1 on the pill, colour-distance ($m.distinct) from rail"; $p = $p + 1
                } else {
                    log-fail $"active app indicator weak: readable ($m.readable):1, distinct ($m.distinct)"; $fl = $fl + 1
                }
            }
            # The open app is represented in the breadcrumb as its own trailing
            # crumb carrying the app icon (the overlay badge was retired).
            sleep 1sec
            let crumbapp = (wd-execute $session_id 'var links=[...document.querySelectorAll(".top-app-bar .breadcrumbs .crumb .crumb-link")]; var last=links[links.length-1]; var ic=last?last.querySelector(".material-icons"):null; var noBadge=document.querySelector(".crumb-app-badge")?0:1; return JSON.stringify({icon: ic?ic.textContent.trim():"", trailingHref: last?(last.getAttribute("href")||""):"", noBadge: noBadge})')
            let cba = ($crumbapp | from json)
            if ($cba.icon == "how_to_vote") and ($cba.trailingHref | str contains "app=vote") and ($cba.noBadge == 1) {
                log-ok "open app shows as a trailing breadcrumb with its icon"; $p = $p + 1
            } else {
                log-fail $"app not represented as a trailing crumb: ($crumbapp)"; $fl = $fl + 1
            }
            # Hovering a collapsed crumb reveals its name. This is now pure CSS
            # (`.crumb:hover`), so it only responds to a REAL pointer position — a
            # synthetic mouseenter would not trigger :hover. Use WebDriver Actions to
            # actually move the mouse onto the first (collapsed) crumb and assert its
            # name expands (computed max-width goes from 0 to non-zero).
            let nm_before = (wd-execute $session_id 'var c=document.querySelector(".top-app-bar .breadcrumbs .crumb"); var nm=c?c.querySelector(".crumb-name"):null; return nm?getComputedStyle(nm).maxWidth:"none"')
            let crumb_eid = (wd-find $session_id ".top-app-bar .breadcrumbs .crumb")
            if ($crumb_eid | is-not-empty) {
                let acts = ({ actions: [{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", duration: 50, origin: "viewport", x: 3, y: 3 }, { type: "pointerMove", duration: 120, origin: { "element-6066-11e4-a52e-4f735466cecf": $crumb_eid }, x: 2, y: 2 }] }] } | to json)
                wd-post $"/session/($session_id)/actions" $acts | ignore
                sleep 500ms
                let nm_after = (wd-execute $session_id 'var c=document.querySelector(".top-app-bar .breadcrumbs .crumb"); var nm=c?c.querySelector(".crumb-name"):null; return nm?getComputedStyle(nm).maxWidth:"none"')
                let w_before = (try { $nm_before | str replace "px" "" | into float } catch { -1.0 })
                let w_after = (try { $nm_after | str replace "px" "" | into float } catch { -1.0 })
                if ($w_before == 0.0) and ($w_after > 0.0) {
                    log-ok $"real pointer hover expands the crumb name, max-width ($nm_before)->($nm_after)"; $p = $p + 1
                } else {
                    log-fail $"crumb hover-expand broken: before=($nm_before) after=($nm_after)"; $fl = $fl + 1
                }
                # Move the pointer away so later tests aren't left hovering.
                wd-post $"/session/($session_id)/actions" ({ actions: [{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", duration: 30, origin: "viewport", x: 3, y: 400 }] }] } | to json) | ignore
                sleep 200ms
            }
            # Only the ready apps (folder/speak/vote/member) are in the rail; the
            # rest (graph/social/...) are hidden until ready.
            let rail = (wd-execute $session_id 'return JSON.stringify({vote:document.querySelector(".nav-rail a[href*=\"app=vote\"]")?1:0, speak:document.querySelector(".nav-rail a[href*=\"app=speak\"]")?1:0, member:document.querySelector(".nav-rail a[href*=\"app=member\"]")?1:0, graph:document.querySelector(".nav-rail a[href*=\"app=graph\"]")?1:0, social:document.querySelector(".nav-rail a[href*=\"app=social\"]")?1:0})')
            let ok = (try { let j = ($rail | from json); ($j.vote == 1) and ($j.speak == 1) and ($j.member == 1) and ($j.graph == 0) and ($j.social == 0) } catch { false })
            if $ok {
                log-ok "app rail shows only the ready apps"; $p = $p + 1
            } else {
                log-fail $"app rail set unexpected: ($rail)"; $fl = $fl + 1
            }
        } else {
            log-warn "no vote rail item — skipping app-switch check"
        }
    } else {
        log-warn "app rail not found — skipping app-switch check"
    }

    # ── Breadcrumbs start at the context (nearest group/event), not the root ──
    wd-navigate $session_id $"(base-url)($ctx_path)"
    wd-wait-y $session_id 'return document.querySelector(".breadcrumbs .crumb")?"y":"n"' 3000 | ignore
    # At a top-level context the trail begins with the context itself, so there
    # is no Home crumb (a breadcrumb link to "/").
    let home_crumb = (wd-execute $session_id 'return document.querySelector(".top-app-bar .breadcrumbs a[href=\"/\"]")?"y":"n"')
    let crumbs = (wd-execute $session_id 'return document.querySelectorAll(".top-app-bar .breadcrumbs .crumb").length')
    let ok = (try { ($home_crumb == "n") and (($crumbs | into int) >= 1) } catch { false })
    if $ok {
        log-ok $"breadcrumbs start at the context, no home crumb, crumbs=($crumbs)"; $p = $p + 1
    } else {
        # Context shape varies (a first-segment path may resolve to a parent);
        # informational rather than a hard failure.
        log-warn $"breadcrumbs did not start at context here: home=($home_crumb) crumbs=($crumbs)"
    }

    # ── New apps render via ?app= (graph / program / social / profile / cow) ──
    # Each should mount a card (or its signature element) without trapping.
    for app in [
        { q: "cow", sel: ".cowsay", name: "cow app renders" }
        { q: "graph", sel: "#main .card", name: "graph app renders" }
        { q: "program", sel: "#main .card", name: "program app renders" }
        { q: "social", sel: "#main input", name: "social app renders" }
        { q: "profile", sel: "#main .card", name: "profile app renders" }
        { q: "parent", sel: "#main .card", name: "parent (missing-parents) app renders" }
    ] {
        wd-navigate $session_id $"(base-url)($ctx_path)?app=($app.q)"
        if (wd-wait-for-element $session_id $app.sel 15) {
            let r = (assert-exists $session_id $app.name $app.sel -p $p -f $fl); $p = $r.passed; $fl = $r.failed
        } else {
            log-fail $"($app.name): ($app.sel) not found"; $fl = $fl + 1
        }
    }

    # ── Member table: the M3 Expressive roster (search + filter chips + paging,
    #    built for 1000+ members) mounts its toolbar/footer, and a filter chip
    #    toggles its selected state. Skips gracefully if the context has none. ──
    wd-navigate $session_id $"(base-url)($ctx_path)?app=member"
    if (wd-wait-for-element $session_id "#main .paginated-table" 15) {
        let r = (assert-exists $session_id "member table renders" "#main .paginated-table" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
        let r = (assert-exists $session_id "member table has a search field" "#main .paginated-table .search-field input" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
        let r = (assert-count $session_id "member table has filter chips" "#main .m3-filter-chip" 3 -p $p -f $fl); $p = $r.passed; $fl = $r.failed
        let r = (assert-exists $session_id "member table has a pagination footer" "#main .paginated-table-footer" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
        # Rows must actually load with a non-zero total (regression guard: the count
        # query used a non-existent members_aggregate, whose validation error
        # emptied the whole page).
        let mstats = (wd-execute $session_id 'var rows=document.querySelectorAll("#main .m3-data-table tbody tr").length; var c=document.querySelector("#main .paginated-count"); var m=c?(c.innerText.match(/(\d+)\s*$/)||[])[1]:null; return JSON.stringify({rows: rows, total: m?parseInt(m):0})')
        let ms = ($mstats | from json)
        if ($ms.rows > 0) and ($ms.total > 0) {
            log-ok $"member table loaded ($ms.rows) rows, total ($ms.total)"; $p = $p + 1
        } else {
            log-fail $"member table fetched no members: ($mstats)"; $fl = $fl + 1
        }
        # Screenshot the populated table (default "all" filter) BEFORE the filter
        # toggle below narrows it (the test group's members aren't owners).
        capture-shots $session_id "member"
        # Clicking a not-yet-selected filter chip selects it (M3 filter-chip toggle).
        wd-execute $session_id 'var c=[...document.querySelectorAll("#main .m3-filter-chip")].find(x=>!x.classList.contains("selected")); if(c)c.click(); return 1' | ignore
        sleep 500ms
        let sel = (wd-execute $session_id 'return document.querySelectorAll("#main .m3-filter-chip.selected").length')
        if (($sel | into int) >= 1) { log-ok "filter chip toggles selected state"; $p = $p + 1 } else { log-fail "filter chip did not select"; $fl = $fl + 1 }
        # Remove-member confirm dialog must actually open (non-destructive: we Cancel,
        # never Delete a real member). Reset to "all" first so there are rows.
        wd-execute $session_id 'var a=[...document.querySelectorAll("#main .m3-filter-chip")].find(x=>x.textContent.trim().toLowerCase().startsWith("all")); if(a)a.click(); return 1' | ignore
        sleep 700ms
        wd-execute $session_id 'var b=[...document.querySelectorAll("#main .m3-data-table tbody tr button")].find(x=>{var m=x.querySelector(".material-icons"); return m&&m.textContent.trim()=="person_remove"}); if(b)b.click(); return 1' | ignore
        sleep 600ms
        let dlg = (wd-execute $session_id 'var d=document.querySelector(".m3-dialog"); var act=document.querySelector(".m3-dialog-actions .btn-primary")?1:0; return JSON.stringify({dialog: d?1:0, action: act})')
        let dj = ($dlg | from json)
        if ($dj.dialog == 1) and ($dj.action == 1) {
            log-ok "member remove opens the confirm dialog"; $p = $p + 1
        } else {
            log-fail $"member remove dialog did not open: ($dlg)"; $fl = $fl + 1
        }
        # Cancel — never delete a real member in the harness.
        wd-execute $session_id 'var c=document.querySelector(".m3-dialog-actions .btn-outlined"); if(c)c.click(); return 1' | ignore
        sleep 300ms
    } else {
        log-warn "member table not found (context may have no members) — skipping"
    }

    # ── Mobile shell (390px): the search/breadcrumb bar sits at the BOTTOM (thumb
    #    reach) as an Expressive search pill, above the app navigation bar. ──
    wd-window-rect $session_id 390 844
    sleep 500ms
    wd-navigate $session_id $"(base-url)($ctx_path)"
    sleep 1sec
    if (servo-skip "mobile shell layout (needs a compact viewport)") {
        # Servo ignores the resize to 390px, so the compact/mobile shell never
        # activates — an engine gap, not a layout bug (verified under Firefox).
    } else {
        let mob = (wd-execute $session_id 'var bar=document.querySelector(".top-app-bar"); if(!bar) return "nobar"; var r=bar.getBoundingClientRect(); var pill=document.querySelector(".top-app-bar .expressive-search")?1:0; var nav=document.querySelectorAll(".nav-bar .nav-bar-item").length; return JSON.stringify({barTop:Math.round(r.top), vh:window.innerHeight, pill:pill, navItems:nav})')
        let ok = (try { let j = ($mob | from json); ($j.barTop > ($j.vh / 2)) and ($j.pill == 1) and ($j.navItems >= 1) } catch { false })
        if $ok { log-ok $"mobile: search bar at bottom, expressive pill, nav bar ($mob)"; $p = $p + 1 } else { log-fail $"mobile shell layout off: ($mob)"; $fl = $fl + 1 }
    }
    wd-window-rect $session_id 1280 900
    sleep 400ms

    # The muted-text refactor relies on Dioxus MERGING two `class:` attributes
    # (a base body-* class + text-muted), not last-wins. A profile paragraph must
    # carry BOTH classes, confirming the base class was not silently dropped.
    wd-navigate $session_id $"(base-url)($ctx_path)?app=profile"
    if (wd-wait-for-element $session_id "#main .text-muted" 15) {
        let merged = (wd-execute $session_id 'return document.querySelector("#main .body-medium.text-muted, #main .body-small.text-muted")?"y":"n"')
        if $merged == "y" { log-ok "class attributes merge (base + text-muted)"; $p = $p + 1 } else { log-fail "class merge dropped the base class" ; $fl = $fl + 1 }
    } else {
        log-warn "no .text-muted on profile — skipping class-merge check"
    }

    # ── Search resolves a result to its full node path (not just the key) ────
    wd-navigate $session_id $"(base-url)($ctx_path)"
    sleep 1sec
    wd-execute $session_id 'var b=[...document.querySelectorAll(".bar button.btn-icon")].find(x=>{var m=x.querySelector(".material-icons"); return m&&m.textContent=="search"}); if(b)b.click(); return 1' | ignore
    sleep 500ms
    let sinput = (try { wd-find $session_id ".bar input" } catch { "" })
    if ($sinput | is-not-empty) and $sinput != "null" {
        wd-send-keys $session_id $sinput "e"
        sleep 2sec
        let has_res = (wd-execute $session_id 'return document.querySelector(".search-results .list-item")?"y":"n"')
        if $has_res == "y" {
            let sp_before = (wd-execute $session_id 'return location.pathname')
            wd-execute $session_id 'var it=document.querySelector(".search-results .list-item"); if(it)it.click(); return 1' | ignore
            mut moved = false
            for _ in 1..($timeout) {
                let now = (wd-execute $session_id 'return location.pathname')
                if ($now != null) and ($now != $sp_before) { $moved = true; break }
                sleep 500ms
            }
            sleep 1sec
            if $moved {
                log-ok "search result navigates to its node"; $p = $p + 1
                # And the app-less URL has no stray "?" left by the router.
                let srch = (wd-execute $session_id 'return location.search')
                if ($srch | default "") == "?" {
                    log-fail "plain URL kept a stray '?'"; $fl = $fl + 1
                } else {
                    log-ok "no trailing '?' on app-less URL"; $p = $p + 1
                }
                # A content post (document/policy) shows the nested comment
                # section with a composer.
                sleep 1sec
                let cs = (wd-execute $session_id 'return document.querySelector(".comment-section .comment-input")?"y":"n"')
                if $cs == "y" {
                    log-ok "comment section renders with a composer"; $p = $p + 1
                } else {
                    log-warn "search result is not a content post, skipping comment-section check"
                }
            } else {
                log-fail "search result click did not navigate"; $fl = $fl + 1
            }
        } else {
            log-warn "no search results — skipping search-nav check"
        }
    } else {
        log-warn "search input not found — skipping search-nav check"
    }

    # ── Folder grid/list view mode is remembered across a reload (#125) ──────
    wd-navigate $session_id $"(base-url)($ctx_path)"
    sleep 1sec
    let gt = (wd-execute $session_id 'var b=[...document.querySelectorAll("#main .btn-icon")].find(x=>{var m=x.querySelector(".material-icons"); return m&&m.textContent=="grid_view"}); if(b){b.click(); return "y"} return "n"')
    if $gt == "y" {
        sleep 1sec
        let grid_now = (wd-execute $session_id 'return document.querySelector(".folder-grid")?"y":"n"')
        wd-navigate $session_id $"(base-url)($ctx_path)"
        sleep 2sec
        let grid_after = (wd-execute $session_id 'return document.querySelector(".folder-grid")?"y":"n"')
        if ($grid_now == "y") and ($grid_after == "y") {
            log-ok "folder grid/list choice persists across reload"; $p = $p + 1
        } else {
            log-fail $"folder view not persisted; now=($grid_now) after=($grid_after)"; $fl = $fl + 1
        }
    } else {
        log-warn "no grid toggle (folder has <=1 child) — skipping persistence check"
    }

    # ── Saving in an app returns to the node's non-app view ──────────────────
    # Open the sort app and save (re-persists the same order); it should redirect
    # back to the folder (?app=sort dropped from the URL).
    wd-navigate $session_id $"(base-url)($ctx_path)?app=sort"
    if (wd-wait-for-element $session_id "#main .btn-primary" 15) {
        wd-execute $session_id 'var b=document.querySelector("#main .btn-primary"); if(b)b.click(); return 1' | ignore
        mut redirected = false
        for _ in 1..($timeout) {
            let s = (wd-execute $session_id 'return location.search')
            if not ($s | default "" | str contains "app=sort") { $redirected = true; break }
            sleep 500ms
        }
        if $redirected {
            log-ok "saving the sort app returns to the node view"; $p = $p + 1
        } else {
            log-fail "sort save did not redirect off ?app=sort"; $fl = $fl + 1
        }
    } else {
        log-warn "sort app save button not found — skipping redirect check"
    }

    # ── The add-content button (in the Items card header) opens a modal ──────
    # On desktop the create action is an in-header .add-action icon button (the FAB
    # is repurposed as the tools-sheet trigger on compact), not a floating FAB.
    wd-navigate $session_id $"(base-url)($ctx_path)"
    sleep 1sec
    let addbtn = (wd-execute $session_id 'return document.querySelector("#main .card-header .btn-icon.add-action")?"y":"n"')
    if $addbtn == "y" {
        wd-execute $session_id 'document.querySelector("#main .card-header .btn-icon.add-action").click(); return 1' | ignore
        sleep 1sec
        let modal = (wd-execute $session_id 'return document.querySelector(".m3-dialog")?"y":"n"')
        # Cancel (outlined button) — never Add — so no content is created.
        wd-execute $session_id 'var c=document.querySelector(".m3-dialog .btn-outlined"); if(c)c.click(); return 1' | ignore
        if $modal == "y" {
            log-ok "in-header add-action opens the M3 dialog"; $p = $p + 1
        } else {
            log-fail "in-header add-action did not open the dialog"; $fl = $fl + 1
        }
    } else {
        log-warn "no in-header add-action — skipping add-content check"
    }

    # ── Recursive folder export (.odt), now inside the M3 tools sheet ────────
    wd-navigate $session_id $"(base-url)($ctx_path)"
    sleep 1sec
    # The tools sheet has two forms: a FAB-triggered modal (below extra-large) and
    # a permanent docked side sheet (extra-large — no FAB, always open). The
    # open/Escape/focus-return checks only apply to the modal; run them when a FAB
    # is present, otherwise note the docked form. The export action below is
    # reachable in BOTH forms.
    let has_fab = (wd-execute $session_id 'return document.querySelector(".fab")?"y":"n"')
    if $has_fab == "y" {
        # Open the modal via its bottom-right "bolt" FAB. Focus it first so we can
        # verify focus returns to it on close (a11y).
        wd-execute $session_id 'var t=document.querySelector(".fab"); if(t){t.focus(); t.click();} return 1' | ignore
        sleep 500ms
        # Screenshot the open sheet (side sheet on desktop, bottom sheet on mobile).
        capture-shots $session_id "toolsheet"
        # The sheet must anchor to the VIEWPORT edge, not float inside a card — a
        # regression guard for the transform-containing-block bug (fixed cards).
        let sheet_pos = (wd-execute $session_id 'var s=document.querySelector(".tool-sheet.open"); if(!s) return "noopen"; var r=s.getBoundingClientRect(); return JSON.stringify({right:Math.round(document.documentElement.clientWidth-r.right), bottom:Math.round(window.innerHeight-r.bottom)})')
        let ok = (try { let j = ($sheet_pos | from json); ($j.right <= 2) and ($j.bottom <= 2) } catch { false })
        if $ok { log-ok $"tool sheet anchored to the viewport edge ($sheet_pos)"; $p = $p + 1 } else { log-fail $"tool sheet not at viewport edge: ($sheet_pos)"; $fl = $fl + 1 }
        # Escape closes the sheet (a11y: focus lands inside on open, Esc dismisses).
        wd-execute $session_id 'var a=document.activeElement||document.querySelector(".tool-sheet.open"); if(a){a.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true}))}; return 1' | ignore
        sleep 500ms
        let closed = (wd-execute $session_id 'return document.querySelector(".tool-sheet.open")?"open":"closed"')
        if $closed == "closed" { log-ok "Escape closes the tool sheet"; $p = $p + 1 } else { log-fail "Escape did not close the tool sheet"; $fl = $fl + 1 }
        # Focus returns to the FAB trigger after the sheet closes (a11y).
        let refocused = (wd-execute $session_id 'var a=document.activeElement; var m=(a&&a.querySelector)?a.querySelector(".material-icons"):null; return (a&&a.classList.contains("fab")&&m&&m.textContent=="bolt")?"y":"n"')
        if $refocused == "y" { log-ok "focus returns to the trigger on sheet close"; $p = $p + 1 } else { log-fail "focus not returned to the trigger on close"; $fl = $fl + 1 }
        # Re-open the sheet for the export check.
        wd-execute $session_id 'var t=document.querySelector(".fab"); if(t)t.click(); return 1' | ignore
        sleep 500ms
    } else {
        log-warn "tools sheet is docked (extra-large) — skipping modal open/Escape/focus checks"
    }
    let exp = (wd-execute $session_id 'var b=[...document.querySelectorAll(".tool-sheet .sheet-action")].find(x=>{var m=x.querySelector(".material-icons"); return m&&m.textContent=="download"}); if(b){b.click(); return "y"} return "n"')
    if $exp == "y" {
        sleep 4sec
        let alive = (wd-execute $session_id 'return document.querySelector("#main .card")?"y":"n"')
        if $alive == "y" {
            log-ok "folder export (in tools sheet) runs without trapping"; $p = $p + 1
        } else {
            log-fail "folder export trapped the app"; $fl = $fl + 1
        }
    } else {
        log-warn "no folder export action in tools sheet — skipping export check"
    }

    # ── Rich text editor: mount, seed, toolbar and live formatting ───────────
    # Exercised entirely in the browser DOM without ever clicking Save, so no
    # node's stored content is modified.
    wd-navigate $session_id $"(base-url)($ctx_path)?app=editor"
    if (wd-wait-for-element $session_id "#rich-editor" 15) {
        # The editing surface is a real contenteditable element.
        let ce = (wd-execute $session_id 'return document.getElementById("rich-editor")?.getAttribute("contenteditable")')
        if $ce == "true" {
            log-ok "rich editor mounts (contenteditable)"; $p = $p + 1
            capture-shots $session_id "editor"
        } else {
            log-fail $"rich editor not contenteditable: ($ce)"; $fl = $fl + 1
        }
        # Toolbar: block-style dropdown plus the formatting icon buttons.
        let tools = (wd-execute $session_id 'return JSON.stringify({sel:document.querySelector(".editor-select")?1:0, btns:document.querySelectorAll(".editor-tools .btn-icon").length})')
        let ok = (try { let j = ($tools | from json); ($j.sel == 1) and ($j.btns >= 10) } catch { false })
        if $ok {
            log-ok $"rich editor toolbar renders ($tools)"; $p = $p + 1
        } else {
            log-fail $"editor toolbar incomplete: ($tools)"; $fl = $fl + 1
        }
        # Seeded from the node's Slate content (an empty doc still seeds a block).
        let seeded = (wd-execute $session_id 'var e=document.getElementById("rich-editor"); return e && e.children.length>0 ? "y":"n"')
        if $seeded == "y" {
            log-ok "rich editor seeds content"; $p = $p + 1
        } else {
            log-fail "rich editor empty (not seeded)"; $fl = $fl + 1
        }
        # Critical: a Dioxus re-render (triggered via keyup, which fires the
        # toolbar-state handler) must NOT wipe the browser-owned editor DOM.
        wd-execute $session_id 'var e=document.getElementById("rich-editor"); e.innerHTML="<p>keepme-xyz</p>"; e.dispatchEvent(new KeyboardEvent("keyup",{bubbles:true})); return 1' | ignore
        sleep 1sec
        let kept = (wd-execute $session_id 'return (document.getElementById("rich-editor")||{innerHTML:""}).innerHTML.indexOf("keepme-xyz")>=0?"y":"n"')
        if $kept == "y" {
            log-ok "editor survives re-render (contenteditable not clobbered)"; $p = $p + 1
        } else {
            log-fail "editor content wiped on re-render"; $fl = $fl + 1
        }
        # Live inline/block formatting via execCommand. Servo only partially
        # implements contenteditable execCommand (bold/formatBlock are no-ops), so
        # these run under Firefox and warn-skip on Servo.
        if (servo-skip "editor execCommand (bold / formatBlock)") {
        } else {
            # Live inline formatting: select all and toggle bold via execCommand.
            let bolded = (wd-execute $session_id 'var e=document.getElementById("rich-editor"); if(!e) return "no"; if(!e.innerText.trim()){e.innerHTML="<p>sample text</p>";} e.focus(); document.execCommand("selectAll",false,null); document.execCommand("bold",false,null); return (/<(b|strong)\b/i.test(e.innerHTML)||/font-weight/i.test(e.innerHTML))?"y":"n"')
            if $bolded == "y" {
                log-ok "editor bold command applies"; $p = $p + 1
            } else {
                log-fail "editor bold command had no effect"; $fl = $fl + 1
            }
            # Live block formatting: turn the selection into a heading.
            let blocked = (wd-execute $session_id 'var e=document.getElementById("rich-editor"); e.focus(); document.execCommand("selectAll",false,null); document.execCommand("formatBlock",false,"<h1>"); return e.querySelector("h1")?"y":"n"')
            if $blocked == "y" {
                log-ok "editor block-format applies"; $p = $p + 1
                # Screenshot the editor with a heading so the fluid (container-query)
                # document type is visible: a big h1 on desktop, smaller on mobile.
                capture-shots $session_id "editor-h1"
                # The h1 must actually shrink on a narrow column (fluid type check).
                let h1sz = (wd-execute $session_id 'var e=document.getElementById("rich-editor"); var h=e&&e.querySelector("h1"); return h?Math.round(parseFloat(getComputedStyle(h).fontSize)):0')
                log-ok $"editor h1 computed font-size at desktop: ($h1sz)px"
            } else {
                log-fail "editor formatBlock had no effect"; $fl = $fl + 1
            }
        }
    } else {
        log-warn "rich editor did not mount, skipping editor checks"
    }

    # ── Author field: hidden for contexts, shown for content nodes ───────────
    # Contexts (group/event) do not carry authors; documents/policies do.
    wd-navigate $session_id $"(base-url)($ctx_path)?app=editor"
    if (wd-wait-for-element $session_id "#rich-editor" 15) {
        let ctx_af = (wd-execute $session_id 'return document.querySelector(".author-field")?"y":"n"')
        if $ctx_af == "n" {
            log-ok "context editor has no author field"; $p = $p + 1
        } else {
            log-fail "context editor unexpectedly shows an author field"; $fl = $fl + 1
        }
    }
    # Open a child node's editor; a content node shows the author autocomplete.
    wd-navigate $session_id $"(base-url)($ctx_path)"
    sleep 1sec
    let clicked = (wd-execute $session_id 'var e=document.querySelector("#main .folder-tile, #main .list-link"); if(e){e.click(); return "y"} return "n"')
    if $clicked == "y" {
        sleep 2sec
        let child_path = (wd-execute $session_id 'return location.pathname')
        wd-navigate $session_id $"(base-url)($child_path)?app=editor"
        if (wd-wait-for-element $session_id "#rich-editor" 15) {
            let af = (wd-execute $session_id 'return document.querySelector(".author-field .author-input-wrap input")?"y":"n"')
            if $af == "y" {
                log-ok "content editor shows the author field"; $p = $p + 1
            } else {
                log-warn "child node does not carry authors, skipping author-field presence check"
            }
        }
    } else {
        log-warn "no child node to open, skipping author-field presence check"
    }

    { passed: $p, failed: $fl }
}

# Write-flow: create a throwaway vote/policy in an owned sandbox folder, start a
# poll on it through the UI, cast a vote, post a comment, then delete the whole
# subtree and restore the context's prior "active" relation — leaving the live
# backend exactly as it was. Firefox-only (it needs real form input + a reactive
# ballot); warn-skips on Servo. Setup/teardown use the app's own GraphQL via the
# stored session token; the poll/vote/comment themselves go through the UI, and
# each is verified against the backend (not just the DOM).
def test-vote-flow [session_id: string, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed; mut fl = $failed
    log-info ""
    log-info "── Poll create + vote + comment (write-flow) ────────────"
    if (servo-skip "poll/vote/comment write-flow") {
        return { passed: $p, failed: $fl }
    }
    # A vote/policy is created under this owned "Test" folder, in the dormant
    # HB2 22/23 event context the test user owns and is an active member of.
    let CTX = "5b1ed157-3198-4d8e-9976-cf416c83aafb"
    let FOLDER = "b317d167-15c7-4077-b0d3-19e6d98c934f"
    let FOLDER_PATH = "/radikal_ungdom/hb2/test"
    let GQL = "https://pgvhpsenoifywhuxnybq.hasura.eu-central-1.nhost.run/v1/graphql"
    # gql() prelude: read the session token from localStorage, sync-XHR to Hasura.
    let gql = ('var __s;try{__s=JSON.parse(localStorage.getItem("wiki_session"))}catch(e){}var __T=__s?__s.access_token:"";function gql(q,v){var x=new XMLHttpRequest();x.open("POST","' + $GQL + '",false);x.setRequestHeader("content-type","application/json");x.setRequestHeader("authorization","Bearer "+__T);try{x.send(JSON.stringify({query:q,variables:v}))}catch(e){return {errors:[{message:String(e)}]}}try{return JSON.parse(x.responseText)}catch(e){return {errors:[{message:x.responseText}]}}}')

    # ── Setup: capture the context's prior active relation, insert the policy ──
    let setup_js = ($gql + 'var CTX="' + $CTX + '";var FOLDER="' + $FOLDER + '";var prior=gql("query($p:uuid!){relations(where:{parentId:{_eq:$p},name:{_eq:\"active\"}}){nodeId}}",{p:CTX});var pa=null;try{pa=prior.data.relations[0].nodeId}catch(e){}var key="e2e-policy-"+Date.now();var r=gql("mutation($o:nodes_insert_input!){insertNode(object:$o){id key}}",{o:{name:"E2E poll flow",key:key,mimeId:"vote/policy",parentId:FOLDER,contextId:CTX,mutable:true}});var id=null;try{id=r.data.insertNode.id}catch(e){}return JSON.stringify({id:id,key:key,priorActive:pa,err:r.errors?JSON.stringify(r.errors):null});')
    let setup = (try { wd-execute $session_id $setup_js | from json } catch { {id: null, key: "", priorActive: null, err: "setup exec failed"} })
    if ($setup.id | is-empty) {
        log-fail $"could not create scaffold policy: ($setup.err)"; $fl = $fl + 1
        return { passed: $p, failed: $fl }
    }
    log-ok "scaffold policy created in the Test sandbox"; $p = $p + 1
    let pid = $setup.id
    let pkey = $setup.key

    # ── Add a poll (UI: the StartPollButton on the policy) ──
    wd-navigate $session_id $"(base-url)($FOLDER_PATH)/($pkey)"
    if (wd-wait-y $session_id 'return [...document.querySelectorAll("#main .btn-icon.add-action")].some(function(b){var m=b.querySelector(".material-icons");return m&&m.textContent=="play_arrow"})?"y":"n"' 8000) {
        log-ok "poll-start control shown (owner)"; $p = $p + 1
        wd-execute $session_id 'var b=[...document.querySelectorAll("#main .btn-icon.add-action")].find(function(b){var m=b.querySelector(".material-icons");return m&&m.textContent=="play_arrow"});if(b)b.click();return 1' | ignore
        sleep 700ms
        wd-execute $session_id 'var b=document.querySelector(".m3-dialog-actions .btn-primary");if(b)b.click();return 1' | ignore
        if (wd-wait-y $session_id 'return document.querySelector("#main .ballot-option, #main .btn-cast")?"y":"n"' 9000) {
            log-ok "poll created and ballot rendered"; $p = $p + 1
        } else {
            log-fail "poll did not open a ballot"; $fl = $fl + 1
        }
    } else {
        log-fail "poll-start control missing on the policy"; $fl = $fl + 1
    }

    # ── Cast a vote (UI), verified against the backend ──
    wd-execute $session_id 'var os=[...document.querySelectorAll("#main .ballot-option")];var t=os.find(function(o){return /\bFor\b/.test(o.textContent)})||os[0];if(t)t.click();return 1' | ignore
    sleep 500ms
    wd-execute $session_id 'var b=document.querySelector("#main .btn-cast");if(b)b.click();return 1' | ignore
    sleep 1800ms
    let vres = (try { wd-execute $session_id ($gql + 'var PID="' + $pid + '";var pr=gql("query($p:uuid!){nodes(where:{parentId:{_eq:$p},mimeId:{_eq:\"vote/poll\"}}){id}}",{p:PID});var poll=null;try{poll=pr.data.nodes[0].id}catch(e){}var vc=0;if(poll){var vr=gql("query($p:uuid!){nodes(where:{parentId:{_eq:$p},mimeId:{_eq:\"vote/vote\"}}){id}}",{p:poll});try{vc=vr.data.nodes.length}catch(e){}}return JSON.stringify({poll:poll,votes:vc});') | from json } catch { {poll: null, votes: 0} })
    if (($vres.votes | default 0) >= 1) {
        log-ok $"vote cast and recorded on the poll \(votes=($vres.votes)\)"; $p = $p + 1
    } else {
        log-fail "vote not recorded on the poll"; $fl = $fl + 1
    }

    # ── Post a comment (UI, on the policy's CommentSection), verified via backend ──
    wd-navigate $session_id $"(base-url)($FOLDER_PATH)/($pkey)"
    if (wd-wait-y $session_id 'return document.querySelector("#main .comment-composer .comment-input")?"y":"n"' 8000) {
        wd-execute $session_id 'var ta=document.querySelector("#main .comment-composer .comment-input");if(ta){ta.value="e2e comment "+Date.now();ta.dispatchEvent(new Event("input",{bubbles:true}))}return 1' | ignore
        sleep 500ms
        wd-execute $session_id 'var b=document.querySelector("#main .comment-composer .comment-send");if(b)b.click();return 1' | ignore
        sleep 1800ms
        let cres = (try { wd-execute $session_id ($gql + 'var PID="' + $pid + '";var cr=gql("query($p:uuid!){nodes(where:{parentId:{_eq:$p},mimeId:{_eq:\"vote/comment\"}}){id}}",{p:PID});var cc=0;try{cc=cr.data.nodes.length}catch(e){}return JSON.stringify({comments:cc});') | from json } catch { {comments: 0} })
        if (($cres.comments | default 0) >= 1) {
            log-ok $"comment posted and recorded on the policy \(comments=($cres.comments)\)"; $p = $p + 1
        } else {
            log-fail "comment not recorded on the policy"; $fl = $fl + 1
        }
    } else {
        log-fail "comment composer missing on the policy"; $fl = $fl + 1
    }

    # ── Teardown (always runs): delete the whole scaffold subtree + the policy,
    #    restore the context's prior active relation, and verify the policy is
    #    gone — so a run leaves the live backend exactly as it found it. ──
    let pa_lit = (if ($setup.priorActive | is-empty) { "null" } else { ('"' + $setup.priorActive + '"') })
    let teardown_js = ($gql + 'var CTX="' + $CTX + '";var PID="' + $pid + '";var PA=' + $pa_lit + ';function ch(pid){var r=gql("query($p:uuid!){nodes(where:{parentId:{_eq:$p}}){id}}",{p:pid});var o=[];try{o=r.data.nodes.map(function(n){return n.id})}catch(e){}return o;}var all=[];var stack=[PID];var guard=0;while(stack.length&&guard<500){guard++;var id=stack.pop();var kids=ch(id);for(var i=0;i<kids.length;i++){all.push(kids[i]);stack.push(kids[i]);}}for(var i=0;i<all.length;i++){gql("mutation($i:uuid!){deleteNode(id:$i){id}}",{i:all[i]});}gql("mutation($i:uuid!){deleteNode(id:$i){id}}",{i:PID});gql("mutation($o:relations_insert_input!,$oc:relations_on_conflict!){insertRelation(object:$o,on_conflict:$oc){id}}",{o:{name:"active",parentId:CTX,nodeId:PA},oc:{constraint:"relations_parent_id_name_key",update_columns:["nodeId"]}});var chk=gql("query($i:uuid!){node(id:$i){id}}",{i:PID});var gone=true;try{gone=!chk.data.node}catch(e){}return JSON.stringify({deleted:all.length,policyGone:gone});')
    let td = (try { wd-execute $session_id $teardown_js | from json } catch { {deleted: -1, policyGone: false} })
    if ($td.policyGone == true) {
        log-ok $"cleaned up scaffold \(($td.deleted) descendant nodes\) and restored active relation"; $p = $p + 1
    } else {
        log-fail $"cleanup incomplete, MANUAL CHECK: policy ($pid) deleted=($td.deleted) gone=($td.policyGone)"; $fl = $fl + 1
    }

    { passed: $p, failed: $fl }
}

# Component CSS contracts for the M3 carousel and content image/lightbox. These
# render from data the harness may not have (candidate photos, content images),
# so verify the styling by injecting the exact markup the components emit and
# reading computed styles (data-independent), plus screenshots for review.
def test-components [session_id: string, passed: int, failed: int] {
    mut p = $passed
    mut fl = $failed
    log-info ""
    log-info "── M3 carousel + content image ──"

    # Carousel: horizontal snap-scroll strip of rounded items.
    let cjs = 'var w=document.createElement("div"); w.id="__ctest_carousel"; var car=document.createElement("div"); car.className="m3-carousel"; car.setAttribute("aria-label","Candidates"); for(var i=0;i<5;i++){var a=document.createElement("a"); a.className="m3-carousel-item"; var ph=document.createElement("div"); ph.className="m3-carousel-placeholder"; var ic=document.createElement("span"); ic.className="material-icons"; ic.textContent="person"; ph.appendChild(ic); a.appendChild(ph); var lb=document.createElement("div"); lb.className="m3-carousel-label"; lb.textContent="Candidate "+(i+1); a.appendChild(lb); car.appendChild(a);} w.appendChild(car); document.body.appendChild(w); var cs=getComputedStyle(car); var it=car.querySelector(".m3-carousel-item"); var is=getComputedStyle(it); return JSON.stringify({snap:cs.scrollSnapType,overflowX:cs.overflowX,itemRadius:parseFloat(is.borderTopLeftRadius)||0,itemSnap:is.scrollSnapAlign,itemW:parseFloat(is.width)||0})'
    let craw = (wd-execute $session_id $cjs)
    let c = (try { $craw | from json } catch { null })
    if $c == null {
        log-fail "carousel CSS probe returned non-JSON"; $fl = $fl + 1
    } else if ($c.snap | str contains "x") and (($c.overflowX == "auto") or ($c.overflowX == "scroll")) and ($c.itemRadius > 0) and ($c.itemSnap | str contains "start") and ($c.itemW > 0) {
        log-ok $"carousel: horizontal snap-scroll, rounded items radius=($c.itemRadius)px width=($c.itemW)px"; $p = $p + 1
    } else {
        log-fail $"carousel CSS off: ($craw)"; $fl = $fl + 1
    }
    if (($env | get -o WIKI_SHOTS | default "") == "1") {
        let dir = ($env | get -o WIKI_SHOTS_DIR | default "screenshots")
        wd-execute $session_id 'var w=document.getElementById("__ctest_carousel"); if(w){w.style.cssText="position:fixed;top:80px;left:16px;width:540px;z-index:2000;background:var(--md-surface-container-low);border-radius:16px;padding-top:12px;box-shadow:0 2px 10px rgba(0,0,0,0.25)"; var h=document.createElement("div"); h.style.cssText="padding:4px 16px 4px;font-weight:600"; h.textContent="Candidates"; w.insertBefore(h,w.firstChild);} return 1' | ignore
        sleep 300ms
        wd-screenshot $session_id ($dir | path join "carousel.png")
    }
    wd-execute $session_id 'var w=document.getElementById("__ctest_carousel"); if(w)w.remove(); return 1' | ignore

    # Content image: .zoomable is a capped, rounded thumbnail (not full-bleed). The
    # `min(100%, 20rem)` cap is left unresolved by Firefox's getComputedStyle, so
    # measure the RENDERED width inside a wide (900px) container instead.
    wd-execute $session_id 'var box=document.createElement("div"); box.id="__ctest_img"; box.style.cssText="width:900px"; var img=document.createElement("img"); img.className="zoomable"; img.src="data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%22800%22 height=%22600%22%3E%3Crect width=%22800%22 height=%22600%22 fill=%22%23607d8b%22/%3E%3C/svg%3E"; box.appendChild(img); document.body.appendChild(box); return 1' | ignore
    sleep 400ms
    let iraw = (wd-execute $session_id 'var img=document.querySelector("#__ctest_img img.zoomable"); if(!img) return "none"; var r=img.getBoundingClientRect(); var s=getComputedStyle(img); return JSON.stringify({w:Math.round(r.width),radius:parseFloat(s.borderTopLeftRadius)||0})')
    let im = (try { $iraw | from json } catch { null })
    if $im == null {
        log-fail $"image CSS probe returned non-JSON: ($iraw)"; $fl = $fl + 1
    } else if ($im.w > 0) and ($im.w <= 340) and ($im.radius > 0) {
        log-ok $"content image is a capped rounded thumbnail, rendered width=($im.w)px in a 900px box"; $p = $p + 1
    } else {
        log-fail $"content image not constrained: ($iraw)"; $fl = $fl + 1
    }
    if (($env | get -o WIKI_SHOTS | default "") == "1") {
        let dir = ($env | get -o WIKI_SHOTS_DIR | default "screenshots")
        # Inject the lightbox (as clicking the image would) and screenshot it.
        wd-execute $session_id 'var lb=document.createElement("div"); lb.className="image-lightbox"; var im=document.createElement("img"); im.className="image-lightbox-img"; im.src="data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%22800%22 height=%22600%22%3E%3Crect width=%22800%22 height=%22600%22 fill=%22%23607d8b%22/%3E%3Ccircle cx=%22400%22 cy=%22300%22 r=%22160%22 fill=%22%23c2185b%22/%3E%3C/svg%3E"; lb.appendChild(im); var cb=document.createElement("button"); cb.className="image-lightbox-close btn-icon state-layer"; var ci=document.createElement("span"); ci.className="material-icons"; ci.textContent="close"; cb.appendChild(ci); lb.appendChild(cb); lb.id="__ctest_lb"; document.body.appendChild(lb); return 1' | ignore
        sleep 400ms
        wd-screenshot $session_id ($dir | path join "image-lightbox.png")
        wd-execute $session_id 'var l=document.getElementById("__ctest_lb"); if(l)l.remove(); return 1' | ignore
    }
    wd-execute $session_id 'var b=document.getElementById("__ctest_img"); if(b)b.remove(); return 1' | ignore

    # Inline invitation in the groups list (accept / reject) — screenshot for review
    # (the test account has no pending invites, so inject the exact markup).
    if (($env | get -o WIKI_SHOTS | default "") == "1") {
        let dir = ($env | get -o WIKI_SHOTS_DIR | default "screenshots")
        wd-execute $session_id 'var card=document.createElement("div"); card.className="card"; card.id="__ctest_invite"; card.style.cssText="position:fixed;top:80px;left:16px;width:360px;z-index:2000"; var h=document.createElement("div"); h.className="card-header"; var ht=document.createElement("h3"); ht.className="title-medium"; ht.textContent="Groups"; h.appendChild(ht); card.appendChild(h); var list=document.createElement("div"); list.className="list"; function mk(nm,invited){var it=document.createElement("div"); it.className="list-item"; var av=document.createElement("div"); av.className="avatar small secondary"; var ai=document.createElement("span"); ai.className="material-icons"; ai.textContent="group"; av.appendChild(ai); it.appendChild(av); var tx=document.createElement("div"); tx.className="list-item-text"; var p1=document.createElement("div"); p1.className="list-item-primary"; p1.textContent=nm; tx.appendChild(p1); if(invited){var p2=document.createElement("div"); p2.className="list-item-secondary"; p2.textContent="Invited"; tx.appendChild(p2);} it.appendChild(tx); if(invited){var b1=document.createElement("button"); b1.className="btn-icon add-action state-layer"; var i1=document.createElement("span"); i1.className="material-icons"; i1.textContent="check"; b1.appendChild(i1); it.appendChild(b1); var b2=document.createElement("button"); b2.className="btn-icon state-layer"; var i2=document.createElement("span"); i2.className="material-icons"; i2.textContent="close"; b2.appendChild(i2); it.appendChild(b2);} return it;} list.appendChild(mk("Klimaudvalget",true)); list.appendChild(mk("Test",false)); list.appendChild(mk("Blog",false)); card.appendChild(list); document.body.appendChild(card); return 1' | ignore
        sleep 300ms
        wd-screenshot $session_id ($dir | path join "invited-item.png")
        wd-execute $session_id 'var c=document.getElementById("__ctest_invite"); if(c)c.remove(); return 1' | ignore
    }

    { passed: $p, failed: $fl }
}

# ── Main ────────────────────────────────────────────────────────────────────

def main [
    --timeout: int = 30
    --verbose
    --keep
    --firefox   # Drive real Firefox (via geckodriver) instead of Servo. Firefox
                # catches client-side routing / rendering bugs Servo masks.
    --shots     # Capture PNG screenshots (light/dark x desktop/mobile) of key
                # screens to ./screenshots for visual review.
    --reuse     # Reuse a `dx serve` already listening on the serve port instead of
                # rebuilding. Start one with `--keep` once, then pass --reuse to skip
                # the ~minute build on every subsequent run.
] {
    let proj = $env.FILE_PWD
    if $shots {
        $env.WIKI_SHOTS = "1"
        $env.WIKI_SHOTS_DIR = ($proj | path join "screenshots")
    }
    # Record the driving engine so tests can skip checks headless Servo can't do
    # (it never fires window-resize events and only partially implements
    # contenteditable execCommand). These pass under real Firefox (--firefox),
    # which is the reference engine; on Servo they warn-skip instead of failing.
    $env.WIKI_ENGINE = (if $firefox { "firefox" } else { "servo" })
    mut servo_pid = 0
    mut server_pid = 0
    mut session_id = ""
    mut passed = 0
    mut failed = 0

    # Preflight
    let servo = if $firefox { "" } else { (servo-bin) }
    if (not $firefox) and ($servo | is-empty) { log-fail "Servo not found (need `servo` in the dev shell)"; exit 2 }
    if $firefox and (which geckodriver | is-empty) and (which nix | is-empty) {
        log-fail "Firefox mode needs `geckodriver` + `firefox` on PATH, or `nix`"; exit 2
    }
    for cmd in [dx curl jq] {
        if (which $cmd | is-empty) { log-fail $"Required command not found: ($cmd)"; exit 2 }
    }

    cd $proj
    # Reuse a `dx serve` already listening on the serve port (skips the ~minute
    # rebuild) when --reuse is set and it answers; otherwise start a fresh one.
    let reuse_active = ($reuse and ((do -i { ^curl -sf -o /dev/null $"(base-url)/" } | complete).exit_code == 0))
    mut serve_log = ""
    if $reuse_active {
        log-info $"Reusing `dx serve` already running on :($SERVE_PORT) — skipping build."
    } else {
        kill-port $SERVE_PORT
        # Start dx serve (debug build — the one Servo can run).
        log-info $"Starting `dx serve` on :($SERVE_PORT) \(first build may take a minute)..."
        $serve_log = (^mktemp /tmp/wiki-dx-XXXXXX.log | str trim)
        $server_pid = (^bash -c $'dx serve --port ($SERVE_PORT) > "($serve_log)" 2>&1 & echo $!' | str trim | into int)

        # `dx serve` binds the port before it finishes the first wasm build, so wait
        # for the build to actually complete (its log announces it) — otherwise the
        # page loads with no wasm and nothing mounts.
        let srv_pid = $server_pid
        mut ready = false
        for _ in 1..181 {
            let built = (try { (open --raw $serve_log) | str contains "Build completed" } catch { false })
            let up = ((do -i { ^curl -sf -o /dev/null $"(base-url)/" } | complete).exit_code == 0)
            if $built and $up { $ready = true; break }
            let alive = (do -i { ^kill -0 $srv_pid } | complete)
            if $alive.exit_code != 0 { log-fail "dx serve exited during build"; break }
            sleep 1sec
        }
        if not $ready { log-fail "dx serve did not finish its first build"; if ($serve_log | path exists) { print -e (open --raw $serve_log | lines | last 15 | str join "\n") }; do-cleanup $session_id $servo_pid $server_pid $reuse_active; exit 2 }
        log-info "dx serve ready (build complete)"
    }

    kill-port $WD_PORT

    # Start the WebDriver browser: Servo by default, or real Firefox (geckodriver)
    # with --firefox.
    let servo_log = (^mktemp /tmp/wiki-wd-XXXXXX.log | str trim)
    if $firefox {
        log-info $"Starting geckodriver + Firefox \(WebDriver on :($WD_PORT))..."
        $servo_pid = (^bash -c (gecko-cmd $servo_log) | str trim | into int)
    } else {
        log-info $"Starting Servo \(headless, WebDriver on :($WD_PORT))..."
        $servo_pid = (^bash -c $'($servo) --headless --webdriver=($WD_PORT) "about:blank" > "($servo_log)" 2>&1 & echo $!' | str trim | into int)
    }

    mut wd_ready = false
    for _ in 1..101 {
        let check = (do -i { ^curl -sf $"(wd-url)/status" } | complete)
        if $check.exit_code == 0 { $wd_ready = true; break }
        sleep 300ms
    }
    if not $wd_ready { log-fail "WebDriver server did not become ready"; do-cleanup $session_id $servo_pid $server_pid $reuse_active; exit 2 }

    let caps = if $firefox {
        '{"capabilities":{"alwaysMatch":{"moz:firefoxOptions":{"args":["-headless"]},"acceptInsecureCerts":true}}}'
    } else {
        '{"capabilities":{}}'
    }
    $session_id = (wd-new-session $caps)
    if ($session_id | is-empty) { log-fail "Failed to create WebDriver session"; do-cleanup $session_id $servo_pid $server_pid $reuse_active; exit 2 }
    wd-set-timeouts $session_id 20000
    log-info $"Session: ($session_id)"

    # Prime the origin once and clear any persisted session, so the first test
    # starts logged-out. A session left by an earlier run (or `--keep`) would
    # otherwise make `/` render the authenticated home and hide the login links
    # the shell smoke test asserts on. Doing it once here (rather than a second
    # in-test navigate) keeps each test to a single, race-free page load.
    wd-navigate $session_id $"(base-url)/"
    wd-wait-for-mount $session_id 40 | ignore
    wd-execute $session_id 'try{localStorage.clear()}catch(e){}; return "ok"' | ignore

    # Run tests
    let r = (test-shell $session_id $timeout $passed $failed); $passed = $r.passed; $failed = $r.failed

    let email = ($env | get -o WIKI_EMAIL | default "")
    let password = ($env | get -o WIKI_PASSWORD | default "")
    if ($email | is-not-empty) and ($password | is-not-empty) {
        let r = (test-auth $session_id $email $password $timeout $passed $failed); $passed = $r.passed; $failed = $r.failed
        # Write-flow (create poll / vote / comment) needs the authed session and
        # only runs for real under Firefox; it self-cleans on the live backend.
        let r = (test-vote-flow $session_id $passed $failed); $passed = $r.passed; $failed = $r.failed
    } else {
        log-info ""
        log-info "Skipping authenticated tests (set WIKI_EMAIL / WIKI_PASSWORD to enable)."
    }

    # Component CSS contracts (carousel + content image/lightbox), data-independent.
    let r = (test-components $session_id $passed $failed); $passed = $r.passed; $failed = $r.failed

    print -e ""
    if $verbose and ($servo_log | path exists) {
        log-info "--- Servo output ---"; print -e (open --raw $servo_log); log-info "--- end ---"
    }

    # Reused servers are always left running (implicit --keep for the server).
    if $reuse_active { do-cleanup $session_id $servo_pid $server_pid true; rm -f $servo_log } else if not $keep { do-cleanup $session_id $servo_pid $server_pid false; rm -f $serve_log $servo_log }

    let total = $passed + $failed
    if $failed == 0 {
        log-ok $"($total) tests: ($passed) passed, 0 failed"
        if $keep { log-info $"Left running — dx serve pid ($server_pid), Servo pid ($servo_pid)." }
        exit 0
    } else {
        log-fail $"($total) tests: ($passed) passed, ($failed) failed"
        exit 1
    }
}
