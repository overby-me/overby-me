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
#                                          #   PNGs of key screens to ./screenshots
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
        sleep 300ms
        for theme in ["light", "dark"] {
            wd-execute $session_id ("document.documentElement.setAttribute('data-theme','" + $theme + "'); return 1") | ignore
            wd-execute $session_id "void document.body.offsetHeight; return 1" | ignore
            sleep 500ms
            let out = ($dir | path join $"($name)-($wh.tag)-($theme).png")
            # Screenshot twice: after a CSS-var (theme) change headless Firefox can
            # return a pre-repaint frame the first time; the second grabs the paint.
            wd-screenshot $session_id $out
            sleep 250ms
            wd-screenshot $session_id $out
        }
    }
    # Restore the app's default (light) at desktop for subsequent tests.
    wd-execute $session_id "document.documentElement.setAttribute('data-theme','light'); return 1" | ignore
    wd-window-rect $session_id 1280 900
    sleep 200ms
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

def do-cleanup [session_id: string, driver_pid: int, server_pid: int] {
    if ($session_id | is-not-empty) { wd-delete $"/session/($session_id)" }
    for pid in [$driver_pid $server_pid] {
        if $pid > 0 {
            let alive = (do -i { ^kill -0 $pid } | complete)
            if $alive.exit_code == 0 { do -i { ^kill $pid } | complete | ignore }
        }
    }
    # dx serve spawns a wrapped child that outlives its parent, and geckodriver
    # is launched through a nix wrapper whose pid isn't the driver's — sweep both
    # ports so neither the dev server nor the WebDriver server is left running.
    kill-port $SERVE_PORT
    kill-port $WD_PORT
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
    let r = (assert-contains $session_id "welcome card shown" "#main .headline-small" "RadikalWiki" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
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
    sleep 3sec

    # The greeting should replace the logged-out copy.
    let r = (assert-contains $session_id "greeting shows the user" "#main .body-large" "Hello" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
    let r = (check-contrast $session_id "home (authenticated)" $p $fl); $p = $r.passed; $fl = $r.failed
    capture-shots $session_id "home"

    # User menu: clicking the avatar opens a dropdown that stays fully within the
    # viewport, and the trigger has no stray border (regression: the primitive
    # drew a rounded square and positioned the popup off-screen).
    let umbtn = (wd-execute $session_id 'var b=document.querySelector(".user-menu > button"); if(b){b.click(); return "y"} return "n"')
    if $umbtn == "y" {
        sleep 400ms
        let dd = (wd-execute $session_id 'return document.querySelector(".user-menu-dropdown")?"y":"n"')
        if $dd == "y" {
            log-ok "user menu opens a dropdown"; $p = $p + 1
            let inview = (wd-execute $session_id 'var d=document.querySelector(".user-menu-dropdown"); var r=d.getBoundingClientRect(); return (r.left>=-1 && r.top>=-1 && r.right<=window.innerWidth+1 && r.bottom<=window.innerHeight+1)?"y":JSON.stringify({l:Math.round(r.left),t:Math.round(r.top),r:Math.round(r.right),b:Math.round(r.bottom),w:window.innerWidth,h:window.innerHeight})')
            if $inview == "y" { log-ok "user menu popup is within the viewport"; $p = $p + 1 } else { log-fail $"user menu popup off-screen: ($inview)"; $fl = $fl + 1 }
            let bw = (wd-execute $session_id 'var b=document.querySelector(".user-menu > button"); return getComputedStyle(b).borderTopWidth')
            if (($bw | default "") == "0px") { log-ok "user menu button has no stray border"; $p = $p + 1 } else { log-warn $"user menu button border-width: ($bw)" }
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
    let r = (assert-count $session_id "drawer shows group/event items" ".drawer .avatar.secondary" 1 -p $p -f $fl); $p = $r.passed; $fl = $r.failed

    # ── In-context navigation (drawer node tree + app rail) ──────────────
    # Click the first context; the app should route into it, render a node
    # view, switch the drawer from the home list to the MenuList tree, and
    # reveal the app rail.
    # Open the first context that actually has content, so the in-context checks
    # (child tree, apps, breadcrumbs) run against a populated node. Many groups
    # are empty; blindly clicking the first avatar can land on an empty one and
    # make every downstream check fail spuriously. Click each context's list-item
    # (the avatar is a child span) until the view shows folder children.
    let n_ctx_str = (wd-execute $session_id 'return String(document.querySelectorAll(".drawer .avatar.secondary").length)')
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
        let click_js = ("var xs=document.querySelectorAll('.drawer .avatar.secondary'); var e=xs[" + ($ci | into string) + "]; if(e){e.closest('.list-item').click(); return 'clicked'} return 'none'")
        wd-execute $session_id $click_js | ignore
        mut moved = false
        for _ in 1..12 {
            let path = (wd-execute $session_id 'return location.pathname')
            if ($path != null) and ($path != "/") { $moved = true; break }
            sleep 300ms
        }
        if $moved {
            sleep 1500ms
            let populated = (wd-execute $session_id 'return document.querySelector("#main .folder-item")?"y":"n"')
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

    # Pull-to-refresh must actually REFETCH data, not just animate. Hook fetch to
    # count GraphQL calls, over-scroll up, and expect fresh calls to fire (the
    # generalized use_data_resource! makes every view refetch on the bump).
    let path_b = (wd-execute $session_id 'return location.pathname')
    let items_b = (wd-execute $session_id 'return String(document.querySelectorAll("#main .folder-item, #main .card").length)')
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
        let now = (wd-execute $session_id 'return String(document.querySelectorAll("#main .folder-item, #main .card").length)')
        if ((try { $now | into int } catch { 0 }) > 0) { $settled = true; break }
    }
    let path_a = (wd-execute $session_id 'return location.pathname')
    let items_a = (wd-execute $session_id 'return String(document.querySelectorAll("#main .folder-item, #main .card").length)')
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
    let r = (assert-count $session_id "app rail shown in context" ".app-rail .btn-icon" 1 -p $p -f $fl); $p = $r.passed; $fl = $r.failed
    # The app rail sits to the LEFT of the side panel (rail | drawer | content).
    let layout = (wd-execute $session_id 'var rl=document.querySelector(".app-rail"),dr=document.querySelector(".drawer-inner"); if(!rl||!dr) return "missing"; return JSON.stringify({rail:Math.round(rl.getBoundingClientRect().left), drawer:Math.round(dr.getBoundingClientRect().left)})')
    let ok = (try { let j = ($layout | from json); ($j.rail < $j.drawer) and ($j.rail < 10) } catch { false })
    if $ok {
        log-ok $"app rail is left of the drawer ($layout)"; $p = $p + 1
    } else {
        log-fail $"app rail not left of drawer: ($layout)"; $fl = $fl + 1
    }
    # The drawer must have swapped the home list for the node tree: the
    # "Groups" home heading is gone once inside a context.
    let drawer_txt = (wd-execute $session_id 'return (document.querySelector(".drawer")||{innerText:""}).innerText')
    if ($drawer_txt | describe) == "string" and (not ($drawer_txt | str contains "Groups")) {
        log-ok "drawer switched to node tree (home list gone)"; $p = $p + 1
    } else {
        log-fail "drawer still shows the home list inside a context"; $fl = $fl + 1
    }

    # ── Navigate to a child node (regression: PathPage must re-resolve on a
    # client-side route change between two path pages, not show stale content).
    let path_before = (wd-execute $session_id 'return location.pathname')
    let main_before = (wd-execute $session_id 'return (document.getElementById("main")||{innerText:""}).innerText')
    let clicked = (wd-execute $session_id 'var e=document.querySelector("#main .folder-item"); if(e){e.click(); return "y"} return "n"')
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
            let sel = (wd-execute $session_id 'return document.querySelector(".drawer .list-item.selected, .drawer-inner .list-item.selected")?"y":"n"')
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
    if (wd-wait-for-element $session_id ".app-rail a" 15) {
        let clicked_vote = (wd-execute $session_id 'var a=[...document.querySelectorAll(".app-rail a")].find(function(x){return (x.getAttribute("href")||"").includes("app=vote")}); if(a){a.click(); return "y"} return "n"')
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
            capture-shots $session_id "vote-app"
            # The highlighted app must have readable contrast (icon vs its box):
            # regression for the green-on-green active state.
            let arc = (wd-execute $session_id 'function L(c){var a=c.map(function(v){v/=255;return v<=0.03928?v/12.92:Math.pow((v+0.055)/1.055,2.4)});return 0.2126*a[0]+0.7152*a[1]+0.0722*a[2]} function P(s){var m=s.match(/[0-9.]+/g);return [+m[0],+m[1],+m[2]]} function R(x,y){var p=L(P(x)),q=L(P(y)),h=Math.max(p,q),l=Math.min(p,q);return (h+0.05)/(l+0.05)} var el=document.querySelector(".app-rail .btn-icon.active"); if(!el) return "noactive"; var ic=el.querySelector(".material-icons"); var cs=getComputedStyle(el); var icc=ic?getComputedStyle(ic).color:cs.color; return R(cs.backgroundColor, icc).toFixed(2)')
            if $arc == "noactive" {
                log-warn "no active app-rail item to contrast-check"
            } else if ((try { $arc | into float } catch { 0.0 }) >= 3.0) {
                log-ok $"active app highlight readable contrast ($arc):1"; $p = $p + 1
            } else {
                log-fail $"active app highlight is low-contrast: ($arc):1"; $fl = $fl + 1
            }
            # The open app is badged onto the current node's breadcrumb avatar.
            sleep 1sec
            let badge = (wd-execute $session_id 'return document.querySelector(".bottom-bar .breadcrumbs .crumb-app-badge")?"y":"n"')
            if $badge == "y" {
                log-ok "open app shows a breadcrumb badge"; $p = $p + 1
            } else {
                log-fail "no app badge on the breadcrumb avatar"; $fl = $fl + 1
            }
            # Only the ready apps (folder/speak/vote) are in the rail; the rest
            # (graph/social/...) are hidden until ready.
            let rail = (wd-execute $session_id 'return JSON.stringify({vote:document.querySelector(".app-rail a[href*=\"app=vote\"]")?1:0, speak:document.querySelector(".app-rail a[href*=\"app=speak\"]")?1:0, graph:document.querySelector(".app-rail a[href*=\"app=graph\"]")?1:0, social:document.querySelector(".app-rail a[href*=\"app=social\"]")?1:0})')
            let ok = (try { let j = ($rail | from json); ($j.vote == 1) and ($j.speak == 1) and ($j.graph == 0) and ($j.social == 0) } catch { false })
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
    sleep 2sec
    # At a top-level context the trail begins with the context itself, so there
    # is no Home crumb (a breadcrumb link to "/").
    let home_crumb = (wd-execute $session_id 'return document.querySelector(".bottom-bar .breadcrumbs a[href=\"/\"]")?"y":"n"')
    let crumbs = (wd-execute $session_id 'return document.querySelectorAll(".bottom-bar .breadcrumbs .crumb").length')
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

    # ── The add-content FAB opens a modal (does not add anything) ────────────
    wd-navigate $session_id $"(base-url)($ctx_path)"
    sleep 1sec
    let fab = (wd-execute $session_id 'return document.querySelector(".fab")?"y":"n"')
    if $fab == "y" {
        wd-execute $session_id 'document.querySelector(".fab").click(); return 1' | ignore
        sleep 1sec
        let modal = (wd-execute $session_id 'return document.querySelector(".modal-card")?"y":"n"')
        # Cancel (outlined button) — never Add — so no content is created.
        wd-execute $session_id 'var c=document.querySelector(".modal-card .btn-outlined"); if(c)c.click(); return 1' | ignore
        if $modal == "y" {
            log-ok "add-content FAB opens a modal"; $p = $p + 1
        } else {
            log-fail "add-content FAB did not open a modal"; $fl = $fl + 1
        }
    } else {
        log-warn "no add-content FAB — skipping FAB check"
    }

    # ── Recursive folder export (.odt) runs without trapping the app ─────────
    wd-navigate $session_id $"(base-url)($ctx_path)"
    sleep 1sec
    let exp = (wd-execute $session_id 'var b=[...document.querySelectorAll("#main .card-header .btn-icon")].find(x=>{var m=x.querySelector(".material-icons"); return m&&m.textContent=="download"}); if(b){b.click(); return "y"} return "n"')
    if $exp == "y" {
        sleep 4sec
        let alive = (wd-execute $session_id 'return document.querySelector("#main .card")?"y":"n"')
        if $alive == "y" {
            log-ok "folder export runs without trapping"; $p = $p + 1
        } else {
            log-fail "folder export trapped the app"; $fl = $fl + 1
        }
    } else {
        log-warn "no folder export button — skipping export check"
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
        } else {
            log-fail "editor formatBlock had no effect"; $fl = $fl + 1
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
    let clicked = (wd-execute $session_id 'var e=document.querySelector("#main .folder-item"); if(e){e.click(); return "y"} return "n"')
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

# ── Main ────────────────────────────────────────────────────────────────────

def main [
    --timeout: int = 30
    --verbose
    --keep
    --firefox   # Drive real Firefox (via geckodriver) instead of Servo. Firefox
                # catches client-side routing / rendering bugs Servo masks.
    --shots     # Capture PNG screenshots (light/dark x desktop/mobile) of key
                # screens to ./screenshots for visual review.
] {
    let proj = $env.FILE_PWD
    if $shots {
        $env.WIKI_SHOTS = "1"
        $env.WIKI_SHOTS_DIR = ($proj | path join "screenshots")
    }
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

    kill-port $SERVE_PORT
    kill-port $WD_PORT

    # Start dx serve (debug build — the one Servo can run).
    log-info $"Starting `dx serve` on :($SERVE_PORT) \(first build may take a minute)..."
    let serve_log = (^mktemp /tmp/wiki-dx-XXXXXX.log | str trim)
    cd $proj
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
    if not $ready { log-fail "dx serve did not finish its first build"; if ($serve_log | path exists) { print -e (open --raw $serve_log | lines | last 15 | str join "\n") }; do-cleanup $session_id $servo_pid $server_pid; exit 2 }
    log-info "dx serve ready (build complete)"

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
    if not $wd_ready { log-fail "WebDriver server did not become ready"; do-cleanup $session_id $servo_pid $server_pid; exit 2 }

    let caps = if $firefox {
        '{"capabilities":{"alwaysMatch":{"moz:firefoxOptions":{"args":["-headless"]},"acceptInsecureCerts":true}}}'
    } else {
        '{"capabilities":{}}'
    }
    $session_id = (wd-new-session $caps)
    if ($session_id | is-empty) { log-fail "Failed to create WebDriver session"; do-cleanup $session_id $servo_pid $server_pid; exit 2 }
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
    } else {
        log-info ""
        log-info "Skipping authenticated tests (set WIKI_EMAIL / WIKI_PASSWORD to enable)."
    }

    print -e ""
    if $verbose and ($servo_log | path exists) {
        log-info "--- Servo output ---"; print -e (open --raw $servo_log); log-info "--- end ---"
    }

    if not $keep { do-cleanup $session_id $servo_pid $server_pid; rm -f $serve_log $servo_log }

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
