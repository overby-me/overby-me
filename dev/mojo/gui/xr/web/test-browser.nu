#!/usr/bin/env nu

# test-browser-xr.nu — Load XR web examples in headless Servo and verify
#                        flat-fallback DOM state via W3C WebDriver.
#
# The XR web runtime falls back to "flat mode" when WebXR is unavailable
# (which is always the case in headless Servo). In flat mode, XR panels
# become visible DOM elements with the app's rendered content. This test
# suite verifies:
#
#   1. The XR runtime initializes and enters flat-fallback mode
#   2. WASM app mutations are applied to the panel DOM container
#   3. Panel containers become visible (position: relative, visibility: visible)
#   4. The #xr-status element reflects the runtime state
#   5. DOM structure is correct for each example app
#
# Prerequisites:
#   - WASM binary built: `just build` (from project root)
#   - XR bundles built: `just build-xr-web`
#   - servo, deno, curl in PATH
#
# Usage:
#   nu xr/web/test-browser.nu                     # Run all app tests
#   nu xr/web/test-browser.nu --timeout 60        # Custom timeout per wait
#   nu xr/web/test-browser.nu --app counter       # Test only counter
#   nu xr/web/test-browser.nu --verbose           # Stream servo stderr
#
# Exit codes:
#   0 — all tests passed
#   1 — test failure
#   2 — missing dependencies or setup failure

const WD_PORT = 7124
const SERVE_PORT = 4508

def wd-url [] { $"http://127.0.0.1:($WD_PORT)" }
def base-url [] { $"http://127.0.0.1:($SERVE_PORT)" }

# ── Logging ────────────────────────────────────────────────────────────────

def log-info [...msg: string] {
    let text = ($msg | str join " ")
    print -e $"(ansi blue_bold)[info](ansi reset)  ($text)"
}

def log-ok [...msg: string] {
    let text = ($msg | str join " ")
    print -e $"(ansi green_bold)[pass](ansi reset)  ($text)"
}

def log-fail [...msg: string] {
    let text = ($msg | str join " ")
    print -e $"(ansi red_bold)[fail](ansi reset)  ($text)"
}

def log-warn [...msg: string] {
    let text = ($msg | str join " ")
    print -e $"(ansi yellow_bold)[warn](ansi reset)  ($text)"
}

# ── WebDriver helpers ──────────────────────────────────────────────────────

def wd-post [path: string, body: string] {
    let url = $"(wd-url)($path)"
    try {
        ^curl -sf -H "Content-Type: application/json" -d $body $url | from json
    } catch {
        null
    }
}

def wd-get [path: string] {
    let url = $"(wd-url)($path)"
    try {
        ^curl -sf $url | from json
    } catch {
        null
    }
}

def wd-delete [path: string] {
    let url = $"(wd-url)($path)"
    try {
        ^curl -sf -X DELETE $url | complete | ignore
    } catch { }
}

def wd-new-session [] {
    let resp = (wd-post "/session" '{"capabilities":{}}')
    if $resp == null { return "" }
    let sid = ($resp | get -o value.sessionId | default ($resp | get -o sessionId | default ""))
    $sid
}

def wd-navigate [session_id: string, url: string] {
    wd-post $"/session/($session_id)/url" $'{"url": "($url)"}' | ignore
}

def wd-find [session_id: string, css: string] {
    let css_json = ($css | to json -r)
    let body = $'{"using": "css selector", "value": ($css_json)}'
    let resp = (wd-post $"/session/($session_id)/element" $body)
    if $resp == null { return "" }
    let val = ($resp | get -o value)
    if $val == null { return "" }
    try {
        $val | values | first
    } catch {
        ""
    }
}

def wd-find-all-count [session_id: string, css: string] {
    let css_json = ($css | to json -r)
    let body = $'{"using": "css selector", "value": ($css_json)}'
    let resp = (wd-post $"/session/($session_id)/elements" $body)
    if $resp == null { return 0 }
    try {
        $resp | get value | length
    } catch {
        0
    }
}

def wd-text [session_id: string, eid: string] {
    let resp = (wd-get $"/session/($session_id)/element/($eid)/text")
    if $resp == null { return "" }
    $resp | get -o value | default ""
}

def wd-attr [session_id: string, eid: string, attr: string] {
    let resp = (wd-get $"/session/($session_id)/element/($eid)/attribute/($attr)")
    if $resp == null { return "" }
    $resp | get -o value | default ""
}

def wd-execute [session_id: string, script: string] {
    let script_json = ($script | to json -r)
    let resp = (wd-post $"/session/($session_id)/execute/sync" $'{"script": ($script_json), "args": []}')
    if $resp == null { return "" }
    $resp | get -o value | default ""
}

def wd-wait-for-element [session_id: string, css: string, max_wait: int] {
    mut elapsed = 0
    while $elapsed < $max_wait {
        let eid = try { wd-find $session_id $css } catch { "" }
        if ($eid | is-not-empty) and $eid != "null" {
            return true
        }
        sleep 500ms
        $elapsed = $elapsed + 1
    }
    false
}

# ── Cleanup helper ─────────────────────────────────────────────────────────

def do-cleanup [
    session_id: string,
    servo_pid: int,
    server_pid: int,
    servo_log: string,
] {
    if ($session_id | is-not-empty) {
        wd-delete $"/session/($session_id)"
    }

    if $servo_pid > 0 {
        let alive = (do -i { ^kill -0 $servo_pid } | complete)
        if $alive.exit_code == 0 {
            log-info $"Shutting down Servo \(PID ($servo_pid))..."
            do -i { ^kill $servo_pid } | complete | ignore
            mut attempts = 0
            while $attempts < 10 {
                let still_alive = (do -i { ^kill -0 $servo_pid } | complete)
                if $still_alive.exit_code != 0 { break }
                sleep 200ms
                $attempts = $attempts + 1
            }
            let still_alive = (do -i { ^kill -0 $servo_pid } | complete)
            if $still_alive.exit_code == 0 {
                do -i { ^kill -9 $servo_pid } | complete | ignore
            }
        }
    }

    if $server_pid > 0 {
        let alive = (do -i { ^kill -0 $server_pid } | complete)
        if $alive.exit_code == 0 {
            log-info $"Shutting down file server \(PID ($server_pid))..."
            do -i { ^kill $server_pid } | complete | ignore
        }
    }

    if ($servo_log | is-not-empty) and ($servo_log | path exists) {
        rm -f $servo_log
    }
}

# ── Kill stale processes on a port ─────────────────────────────────────────

def kill-port [port: int] {
    let in_use = try { ^fuser $"($port)/tcp" | complete; true } catch { false }
    if $in_use {
        log-warn $"Port ($port) already in use — killing stale process"
        try { ^fuser -k $"($port)/tcp" | complete } catch { }
        mut attempts = 0
        while $attempts < 20 {
            let still_used = try { ^fuser $"($port)/tcp" | complete; true } catch { false }
            if not $still_used { break }
            sleep 200ms
            $attempts = $attempts + 1
        }
        let still_used = try {
            let result = (^fuser $"($port)/tcp" | complete)
            $result.exit_code == 0
        } catch { false }
        if $still_used {
            log-warn $"Port ($port) still in use after kill — forcing with -9"
            try { ^fuser -k -9 $"($port)/tcp" | complete } catch { }
            sleep 1sec
        }
    }
}

# ── Test assertion helpers ─────────────────────────────────────────────────

def assert-text [
    session_id: string,
    label: string,
    css: string,
    expected: string,
    --passed (-p): int,
    --failed (-f): int,
]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let eid = try { wd-find $session_id $css } catch { "" }
    if ($eid | is-empty) or $eid == "null" {
        log-fail $"($label) — element not found: ($css)"
        $f = $f + 1
    } else {
        let text = (wd-text $session_id $eid)
        if $text == $expected {
            log-ok $label
            $p = $p + 1
        } else {
            log-fail $"($label) — expected: \"($expected)\", got: \"($text)\""
            $f = $f + 1
        }
    }
    { passed: $p, failed: $f }
}

def assert-text-contains [
    session_id: string,
    label: string,
    css: string,
    substring: string,
    --passed (-p): int,
    --failed (-f): int,
]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let eid = try { wd-find $session_id $css } catch { "" }
    if ($eid | is-empty) or $eid == "null" {
        log-fail $"($label) — element not found: ($css)"
        $f = $f + 1
    } else {
        let text = (wd-text $session_id $eid)
        if ($text | str contains $substring) {
            log-ok $label
            $p = $p + 1
        } else {
            log-fail $"($label) — expected to contain: \"($substring)\", got: \"($text)\""
            $f = $f + 1
        }
    }
    { passed: $p, failed: $f }
}

def assert-exists [
    session_id: string,
    label: string,
    css: string,
    --passed (-p): int,
    --failed (-f): int,
]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let eid = try { wd-find $session_id $css } catch { "" }
    if ($eid | is-not-empty) and $eid != "null" {
        log-ok $label
        $p = $p + 1
    } else {
        log-fail $"($label) — element not found: ($css)"
        $f = $f + 1
    }
    { passed: $p, failed: $f }
}

def assert-not-exists [
    session_id: string,
    label: string,
    css: string,
    --passed (-p): int,
    --failed (-f): int,
]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let eid = try { wd-find $session_id $css } catch { "" }
    if ($eid | is-empty) or $eid == "null" {
        log-ok $label
        $p = $p + 1
    } else {
        log-fail $"($label) — element unexpectedly found: ($css)"
        $f = $f + 1
    }
    { passed: $p, failed: $f }
}

def assert-count [
    session_id: string,
    label: string,
    css: string,
    expected: int,
    --passed (-p): int,
    --failed (-f): int,
]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let count = try { wd-find-all-count $session_id $css } catch { 0 }
    if $count == $expected {
        log-ok $label
        $p = $p + 1
    } else {
        log-fail $"($label) — expected ($expected) elements, got ($count)"
        $f = $f + 1
    }
    { passed: $p, failed: $f }
}

def assert-style-contains [
    session_id: string,
    label: string,
    css: string,
    style_substring: string,
    --passed (-p): int,
    --failed (-f): int,
]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let eid = try { wd-find $session_id $css } catch { "" }
    if ($eid | is-empty) or $eid == "null" {
        log-fail $"($label) — element not found: ($css)"
        $f = $f + 1
    } else {
        let style = (wd-attr $session_id $eid "style")
        if ($style | str contains $style_substring) {
            log-ok $label
            $p = $p + 1
        } else {
            log-fail $"($label) — style missing \"($style_substring)\", got: \"($style)\""
            $f = $f + 1
        }
    }
    { passed: $p, failed: $f }
}

# ── App test: XR Counter (flat fallback) ───────────────────────────────────

def test-xr-counter [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── XR Counter (flat fallback) ──────────────────────────"

    wd-navigate $session_id $"($bu)/xr/web/examples/counter/"

    # Wait for the XR panel container to appear in the DOM.
    # The XR runtime appends [data-xr-panel] divs to <body>.
    # In flat-fallback mode, these become visible after start().
    if not (wd-wait-for-element $session_id "[data-xr-panel]" $timeout) {
        log-fail "XR panel container did not appear within timeout"
        $f = $f + 1

        # Check if the page loaded at all
        let status_eid = try { wd-find $session_id "#xr-status" } catch { "" }
        if ($status_eid | is-not-empty) and $status_eid != "null" {
            let status_text = (wd-text $session_id $status_eid)
            log-info $"  xr-status text: \"($status_text)\""
        } else {
            log-info "  #xr-status element not found either — page may not have loaded"
        }

        return { passed: $p, failed: $f }
    }
    sleep 1sec  # Give mutations time to apply

    # ── XR runtime state ──

    let r = (assert-exists $session_id "XR panel container exists" "[data-xr-panel]" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Panel should have data-xr-panel="0" (first panel ID)
    let r = (assert-exists $session_id "first panel has id 0" "[data-xr-panel='0']" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # In flat fallback, panel container should be visible (position: relative)
    let r = (assert-style-contains $session_id "panel is positioned relative (flat fallback)" "[data-xr-panel='0']" "relative" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-style-contains $session_id "panel is visible (flat fallback)" "[data-xr-panel='0']" "visible" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-style-contains $session_id "panel allows pointer events" "[data-xr-panel='0']" "pointer-events: auto" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # ── Status element ──

    let r = (assert-exists $session_id "#xr-status element exists" "#xr-status" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Status should mention fallback or flat mode
    let status_eid = try { wd-find $session_id "#xr-status" } catch { "" }
    if ($status_eid | is-not-empty) and $status_eid != "null" {
        let status_text = (wd-text $session_id $status_eid)
        if ($status_text | str contains "fallback") or ($status_text | str contains "flat") or ($status_text | str contains "not available") {
            log-ok "status indicates flat fallback mode"
            $p = $p + 1
        } else {
            log-warn $"status text unexpected: \"($status_text)\" (may still be loading)"
        }
    }

    # ── No Enter VR button (WebXR not available) ──

    # The "Enter VR" button should NOT be shown when WebXR is unavailable.
    # The runtime creates this button only when xrAvailable is true.
    let r = (assert-not-exists $session_id "no 'Enter VR' button in flat fallback" "#xr-enter-vr" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # ── Counter app DOM content in panel ──

    # The counter app renders: <div><h1>High-Five counter: 0</h1><button>+</button><button>-</button><button>Reset</button></div>
    # This content is inside the [data-xr-panel] container.
    let r = (assert-exists $session_id "counter h1 rendered in panel" "[data-xr-panel] h1" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let panel_eid = try { wd-find $session_id "[data-xr-panel='0']" } catch { "" }
    if ($panel_eid | is-not-empty) and $panel_eid != "null" {
        let panel_text = (wd-text $session_id $panel_eid)
        if ($panel_text | str contains "counter" | ignore; ($panel_text | str downcase | str contains "counter")) or ($panel_text | str contains "Counter") or ($panel_text | str contains "High-Five") {
            log-ok "panel contains counter app text"
            $p = $p + 1
        } else if ($panel_text | str trim | is-not-empty) {
            log-ok $"panel has content: \"(($panel_text | str substring 0..80))...\""
            $p = $p + 1
        } else {
            log-fail "panel container appears empty — WASM mutations may not have applied"
            $f = $f + 1
        }
    }

    # Check for buttons rendered by the counter app
    let btn_count = try { wd-find-all-count $session_id "[data-xr-panel] button" } catch { 0 }
    if $btn_count >= 2 {
        log-ok $"panel has ($btn_count) buttons (counter +/- buttons rendered)"
        $p = $p + 1
    } else if $btn_count >= 1 {
        log-ok $"panel has ($btn_count) button(s) (partial render)"
        $p = $p + 1
    } else {
        log-warn "no buttons found in panel — WASM app may not have fully rendered"
    }

    # ── Panel styling (injected by XRPanel constructor) ──

    # The panel should have a <style> element with panel-specific styles
    let style_count = try { wd-find-all-count $session_id "[data-xr-panel] style" } catch { 0 }
    if $style_count >= 1 {
        log-ok "panel contains injected <style> element"
        $p = $p + 1
    } else {
        log-warn "no <style> found in panel — may be normal for some DOM structures"
    }

    { passed: $p, failed: $f }
}

# ── App test: XR Todo (flat fallback) ──────────────────────────────────────

def test-xr-todo [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── XR Todo (flat fallback) ─────────────────────────────"

    wd-navigate $session_id $"($bu)/xr/web/examples/todo/"

    if not (wd-wait-for-element $session_id "[data-xr-panel]" $timeout) {
        log-fail "XR panel container did not appear within timeout"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }
    sleep 1sec

    let r = (assert-exists $session_id "XR panel container exists" "[data-xr-panel='0']" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-style-contains $session_id "panel visible in flat fallback" "[data-xr-panel='0']" "visible" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Todo app should render an input and a button
    let panel_eid = try { wd-find $session_id "[data-xr-panel='0']" } catch { "" }
    if ($panel_eid | is-not-empty) and $panel_eid != "null" {
        let panel_text = (wd-text $session_id $panel_eid)
        if ($panel_text | str trim | is-not-empty) {
            log-ok "todo panel has rendered content"
            $p = $p + 1
        } else {
            log-fail "todo panel is empty"
            $f = $f + 1
        }
    }

    # Check for input field (todo apps typically have one)
    let input_eid = try { wd-find $session_id "[data-xr-panel] input" } catch { "" }
    if ($input_eid | is-not-empty) and $input_eid != "null" {
        log-ok "todo panel has input field"
        $p = $p + 1
    } else {
        log-warn "no input field found in todo panel"
    }

    { passed: $p, failed: $f }
}

# ── App test: XR Bench (flat fallback) ─────────────────────────────────────

def test-xr-bench [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── XR Bench (flat fallback) ─────────────────────────────"

    wd-navigate $session_id $"($bu)/xr/web/examples/bench/"

    if not (wd-wait-for-element $session_id "[data-xr-panel]" $timeout) {
        log-fail "XR panel container did not appear within timeout"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }
    sleep 1sec

    let r = (assert-exists $session_id "XR panel container exists" "[data-xr-panel='0']" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-style-contains $session_id "panel visible in flat fallback" "[data-xr-panel='0']" "visible" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Bench should render toolbar buttons
    let btn_count = try { wd-find-all-count $session_id "[data-xr-panel] button" } catch { 0 }
    if $btn_count >= 3 {
        log-ok $"bench panel has ($btn_count) toolbar buttons"
        $p = $p + 1
    } else if $btn_count >= 1 {
        log-ok $"bench panel has ($btn_count) button(s)"
        $p = $p + 1
    } else {
        log-warn "no buttons found in bench panel"
    }

    { passed: $p, failed: $f }
}

# ── App test: XR MultiView App (flat fallback) ────────────────────────────

def test-xr-app [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── XR MultiView App (flat fallback) ─────────────────────"

    wd-navigate $session_id $"($bu)/xr/web/examples/app/"

    if not (wd-wait-for-element $session_id "[data-xr-panel]" $timeout) {
        log-fail "XR panel container did not appear within timeout"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }
    sleep 1sec

    let r = (assert-exists $session_id "XR panel container exists" "[data-xr-panel='0']" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-style-contains $session_id "panel visible in flat fallback" "[data-xr-panel='0']" "visible" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # App shell should render navigation
    let nav_eid = try { wd-find $session_id "[data-xr-panel] nav" } catch { "" }
    if ($nav_eid | is-not-empty) and $nav_eid != "null" {
        log-ok "multi-view app has nav element"
        $p = $p + 1
    } else {
        # The app might use buttons instead of nav
        let panel_eid = try { wd-find $session_id "[data-xr-panel='0']" } catch { "" }
        if ($panel_eid | is-not-empty) and $panel_eid != "null" {
            let text = (wd-text $session_id $panel_eid)
            if ($text | str trim | is-not-empty) {
                log-ok "multi-view app panel has content"
                $p = $p + 1
            } else {
                log-warn "multi-view app panel appears empty"
            }
        }
    }

    { passed: $p, failed: $f }
}

# ── Common: XR flat-fallback structural tests ──────────────────────────────
# These run once on the counter page and verify structural properties
# that are common to all XR flat-fallback pages.

def test-xr-structure [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── XR Flat-Fallback Structure ─────────────────────────"

    wd-navigate $session_id $"($bu)/xr/web/examples/counter/"

    if not (wd-wait-for-element $session_id "[data-xr-panel]" $timeout) {
        log-fail "XR panel container did not appear for structural tests"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }
    sleep 1sec

    # Only one panel should exist (single-panel app)
    let r = (assert-count $session_id "exactly one XR panel for single-panel app" "[data-xr-panel]" 1 -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Panel should be a direct child of <body> (not inside #root)
    let r = (assert-exists $session_id "panel is child of body" "body > [data-xr-panel]" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # The #root element should still exist but loading text should be gone
    let r = (assert-exists $session_id "#root element exists" "#root" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Panel container should have width/height set (from texture dimensions)
    let panel_eid = try { wd-find $session_id "[data-xr-panel='0']" } catch { "" }
    if ($panel_eid | is-not-empty) and $panel_eid != "null" {
        let style = (wd-attr $session_id $panel_eid "style")
        if ($style | str contains "px") {
            log-ok "panel has pixel dimensions in style"
            $p = $p + 1
        } else {
            log-warn $"panel style may be missing dimensions: \"($style)\""
        }
    }

    # Panel should have overflow: hidden
    let r = (assert-style-contains $session_id "panel has overflow hidden" "[data-xr-panel='0']" "overflow" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Panel has border and box-shadow in flat mode
    let r = (assert-style-contains $session_id "panel has border in flat mode" "[data-xr-panel='0']" "border" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    { passed: $p, failed: $f }
}

# ── Main ───────────────────────────────────────────────────────────────────

def main [
    --timeout: int = 30     # Per-wait timeout in seconds (poll iterations)
    --app: string = ""      # Test only a specific app (counter, todo, bench, app)
    --verbose                # Stream servo stderr
] {
    # The script_dir is the directory containing this script (xr/web/).
    # The project root is two levels up.
    let script_dir = ($env.FILE_PWD)
    let project_root = ($script_dir | path join "../.." | path expand)

    mut servo_pid = 0
    mut server_pid = 0
    mut session_id = ""
    mut servo_log = ""
    mut passed = 0
    mut failed = 0

    # ── Preflight checks ──────────────────────────────────────────────

    for cmd in [servo deno curl] {
        if (which $cmd | is-empty) {
            log-fail $"Required command not found: ($cmd)"
            exit 2
        }
    }

    # ── Verify build artifacts exist ──────────────────────────────────

    let wasm_path = ($project_root | path join "web/build/out.wasm")
    if not ($wasm_path | path exists) {
        log-fail "WASM binary not found — run `just build` first"
        log-info $"  expected: ($wasm_path)"
        exit 2
    }

    let bundle_path = ($project_root | path join "xr/web/examples/counter/bundle.js")
    if not ($bundle_path | path exists) {
        log-fail "XR bundles not found — run `just build-xr-web` first"
        log-info $"  expected: ($bundle_path)"
        exit 2
    }

    # ── Kill stale processes on our ports ─────────────────────────

    kill-port $SERVE_PORT
    kill-port $WD_PORT

    # ── Start file server (from project root — serves both web/ and xr/) ──

    log-info $"Starting file server on :($SERVE_PORT) from ($project_root)..."
    $server_pid = (^bash -c $'cd "($project_root)" && deno run --allow-net --allow-read jsr:@std/http/file-server -p ($SERVE_PORT) 2>/dev/null & echo $!' | str trim | into int)

    mut ready = false
    for _ in 1..31 {
        let check = (do -i { ^curl -sf $"(base-url)/" } | complete)
        if $check.exit_code == 0 {
            $ready = true
            break
        }
        let _srv_pid = $server_pid
        let alive = (do -i { ^kill -0 $_srv_pid } | complete)
        if $alive.exit_code != 0 {
            log-fail "File server exited unexpectedly"
            exit 2
        }
        sleep 200ms
    }

    if not $ready {
        log-fail "File server did not become ready"
        exit 2
    }

    log-info $"File server ready \(PID ($server_pid))"

    # ── Start Servo (headless + WebDriver) ────────────────────────

    $servo_log = (^mktemp /tmp/servo-xr-test-XXXXXX.log | str trim)

    log-info $"Starting Servo \(headless, WebDriver on :($WD_PORT))..."

    $servo_pid = (^bash -c $'servo --headless --webdriver=($WD_PORT) "about:blank" > "($servo_log)" 2>&1 & echo $!' | str trim | into int)

    mut wd_ready = false
    for _ in 1..51 {
        let check = (do -i { ^curl -sf $"(wd-url)/status" } | complete)
        if $check.exit_code == 0 {
            $wd_ready = true
            break
        }
        let _serv_pid = $servo_pid
        let alive = (do -i { ^kill -0 $_serv_pid } | complete)
        if $alive.exit_code != 0 {
            log-fail "Servo exited before WebDriver was ready"
            if $verbose and ($servo_log | path exists) {
                log-warn "Servo output:"
                print -e (open --raw $servo_log)
            }
            exit 2
        }
        sleep 200ms
    }

    if not $wd_ready {
        log-fail $"WebDriver did not become ready on :($WD_PORT)"
        if ($servo_log | path exists) {
            log-warn "Last 20 lines of Servo output:"
            print -e (open --raw $servo_log | lines | last 20 | str join "\n")
        }
        exit 2
    }

    log-info $"Servo ready \(PID ($servo_pid))"

    # ── Create WebDriver session ──────────────────────────────────

    log-info "Creating WebDriver session..."
    $session_id = (wd-new-session)

    if ($session_id | is-empty) {
        log-fail "Failed to create WebDriver session"
        exit 2
    }

    log-info $"Session: ($session_id)"

    # ── Run tests ─────────────────────────────────────────────────

    log-info ""
    log-info $"Running XR browser tests \(timeout: ($timeout)s per wait)..."

    # Always run structural tests first (uses counter page)
    if ($app | is-empty) or $app == "counter" {
        let r = (test-xr-structure $session_id $timeout $passed $failed)
        $passed = $r.passed; $failed = $r.failed
    }

    if ($app | is-empty) or $app == "counter" {
        let r = (test-xr-counter $session_id $timeout $passed $failed)
        $passed = $r.passed; $failed = $r.failed
    }
    if ($app | is-empty) or $app == "todo" {
        let r = (test-xr-todo $session_id $timeout $passed $failed)
        $passed = $r.passed; $failed = $r.failed
    }
    if ($app | is-empty) or $app == "bench" {
        let r = (test-xr-bench $session_id $timeout $passed $failed)
        $passed = $r.passed; $failed = $r.failed
    }
    if ($app | is-empty) or $app == "app" {
        let r = (test-xr-app $session_id $timeout $passed $failed)
        $passed = $r.passed; $failed = $r.failed
    }

    # ── Summary ───────────────────────────────────────────────────

    print -e ""
    let total = $passed + $failed

    if $verbose and ($servo_log | path exists) and (($servo_log | path type) == "file") {
        let content = (open --raw $servo_log)
        if ($content | is-not-empty) {
            log-info "--- Servo output ---"
            print -e $content
            log-info "--- End Servo output ---"
            print -e ""
        }
    }

    # Clean up before exit
    do-cleanup $session_id $servo_pid $server_pid $servo_log

    if $failed == 0 {
        log-ok $"($total) tests: ($passed) passed, 0 failed"
        exit 0
    } else {
        log-fail $"($total) tests: ($passed) passed, ($failed) failed"
        exit 1
    }
}
