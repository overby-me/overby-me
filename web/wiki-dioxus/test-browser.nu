#!/usr/bin/env nu

# test-browser.nu — Load the RadikalWiki Dioxus app in headless Servo and verify
#                   DOM state via W3C WebDriver, driven from nushell + curl + jq.
#
# Adapted from mojo/gui/web/test-browser.nu. The app is a single Dioxus/WASM SPA
# served by `dx serve` (the debug build is the one that runs in Servo — see
# README.md). Servo exposes a built-in WebDriver server we drive over HTTP.
#
# Usage:
#   nu test-browser.nu                     # Unauthenticated smoke tests
#   nu test-browser.nu --timeout 30        # Per-wait timeout (seconds)
#   nu test-browser.nu --verbose           # Print Servo stderr at the end
#   nu test-browser.nu --keep              # Keep dx serve + Servo running after
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

def wd-new-session [] {
    let resp = (wd-post "/session" '{"capabilities":{}}')
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

def do-cleanup [session_id: string, servo_pid: int, server_pid: int] {
    if ($session_id | is-not-empty) { wd-delete $"/session/($session_id)" }
    for pid in [$servo_pid $server_pid] {
        if $pid > 0 {
            let alive = (do -i { ^kill -0 $pid } | complete)
            if $alive.exit_code == 0 { do -i { ^kill $pid } | complete | ignore }
        }
    }
    # dx serve spawns a wrapped child that outlives its parent; sweep the port.
    kill-port $SERVE_PORT
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
    # Groups + events render as context items (avatar badges) in the drawer.
    let r = (assert-count $session_id "drawer shows group/event items" ".drawer .avatar.secondary" 1 -p $p -f $fl); $p = $r.passed; $fl = $r.failed

    { passed: $p, failed: $fl }
}

# ── Main ────────────────────────────────────────────────────────────────────

def main [
    --timeout: int = 30
    --verbose
    --keep
] {
    let proj = $env.FILE_PWD
    mut servo_pid = 0
    mut server_pid = 0
    mut session_id = ""
    mut passed = 0
    mut failed = 0

    # Preflight
    let servo = (servo-bin)
    if ($servo | is-empty) { log-fail "Servo not found (need `servo` in the dev shell)"; exit 2 }
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

    # Start Servo (headless + WebDriver).
    log-info $"Starting Servo \(headless, WebDriver on :($WD_PORT))..."
    let servo_log = (^mktemp /tmp/wiki-servo-XXXXXX.log | str trim)
    $servo_pid = (^bash -c $'($servo) --headless --webdriver=($WD_PORT) "about:blank" > "($servo_log)" 2>&1 & echo $!' | str trim | into int)

    mut wd_ready = false
    for _ in 1..51 {
        let check = (do -i { ^curl -sf $"(wd-url)/status" } | complete)
        if $check.exit_code == 0 { $wd_ready = true; break }
        sleep 200ms
    }
    if not $wd_ready { log-fail "Servo WebDriver did not become ready"; do-cleanup $session_id $servo_pid $server_pid; exit 2 }

    $session_id = (wd-new-session)
    if ($session_id | is-empty) { log-fail "Failed to create WebDriver session"; do-cleanup $session_id $servo_pid $server_pid; exit 2 }
    wd-set-timeouts $session_id 20000
    log-info $"Session: ($session_id)"

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
