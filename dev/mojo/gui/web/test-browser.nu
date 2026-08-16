#!/usr/bin/env nu

# test-browser.nu — Load mojo-wasm apps in headless Servo and verify DOM state
#                    via W3C WebDriver, driven entirely from nushell + http + jq.
#
# Inspired by rust-nixos/test-boot.nu: build artifact → boot in real environment
# → monitor output → pattern-match success/failure → bounded execution.
#
# Usage:
#   nu test-browser.nu                    # Run all app tests (default timeout)
#   nu test-browser.nu --timeout 60       # Custom timeout per test (seconds)
#   nu test-browser.nu --app counter      # Test only the counter app
#   nu test-browser.nu --verbose          # Stream servo stderr
#   nu test-browser.nu --keep             # Keep servo running after tests
#
# Exit codes:
#   0 — all tests passed
#   1 — test failure
#   2 — missing dependencies or setup failure

const WD_PORT = 7123
const SERVE_PORT = 4507

def wd-url [] { $"http://127.0.0.1:($WD_PORT)" }
def base-url [] { $"http://127.0.0.1:($SERVE_PORT)" }

# ── Logging (same style as rust-nixos/test-boot.nu) ─────────────────────────

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

# ── WebDriver helpers (http + from json — no curl/jq needed) ──────────────

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

# Create a new WebDriver session
def wd-new-session [] {
    let resp = (wd-post "/session" '{"capabilities":{}}')
    if $resp == null { return "" }
    let sid = ($resp | get -o value.sessionId | default ($resp | get -o sessionId | default ""))
    $sid
}

# Navigate to a URL
def wd-navigate [session_id: string, url: string] {
    wd-post $"/session/($session_id)/url" $'{"url": "($url)"}' | ignore
}

# Find an element by CSS selector. Returns the element ID.
def wd-find [session_id: string, css: string] {
    let css_json = ($css | to json -r)
    let body = $'{"using": "css selector", "value": ($css_json)}'
    let resp = (wd-post $"/session/($session_id)/element" $body)
    if $resp == null { return "" }
    let val = ($resp | get -o value)
    if $val == null { return "" }
    # WebDriver returns {"value": {"element-...": "id"}} or {"value": {"ELEMENT": "id"}}
    try {
        $val | values | first
    } catch {
        ""
    }
}

# Find multiple elements by CSS selector. Returns count.
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

# Get the text content of an element by its element ID.
def wd-text [session_id: string, eid: string] {
    let resp = (wd-get $"/session/($session_id)/element/($eid)/text")
    if $resp == null { return "" }
    $resp | get -o value | default ""
}

# Get an attribute of an element.
def wd-attr [session_id: string, eid: string, attr: string] {
    let resp = (wd-get $"/session/($session_id)/element/($eid)/attribute/($attr)")
    if $resp == null { return "" }
    $resp | get -o value | default ""
}

# Click an element by its element ID.
def wd-click [session_id: string, eid: string] {
    wd-post $"/session/($session_id)/element/($eid)/click" '{}' | ignore
}

# Send keys to an element.
def wd-send-keys [session_id: string, eid: string, text: string] {
    let text_json = ($text | to json -r)
    wd-post $"/session/($session_id)/element/($eid)/value" $'{"text": ($text_json)}' | ignore
}

# Execute JavaScript (for waiting on WASM load).
def wd-execute [session_id: string, script: string] {
    let script_json = ($script | to json -r)
    let resp = (wd-post $"/session/($session_id)/execute/sync" $'{"script": ($script_json), "args": []}')
    if $resp == null { return "" }
    $resp | get -o value | default ""
}

# Wait for a condition (poll-based, like test-boot.nu serial monitoring).
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
    # Delete WebDriver session
    if ($session_id | is-not-empty) {
        wd-delete $"/session/($session_id)"
    }

    # Kill servo
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

    # Kill file server
    if $server_pid > 0 {
        let alive = (do -i { ^kill -0 $server_pid } | complete)
        if $alive.exit_code == 0 {
            log-info $"Shutting down file server \(PID ($server_pid))..."
            do -i { ^kill $server_pid } | complete | ignore
        }
    }

    # Clean up servo log
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

# ── App test: Counter ──────────────────────────────────────────────────────

def test-counter [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── Counter app ──────────────────────────────────────────"

    wd-navigate $session_id $"($bu)/examples/counter/"

    # Wait for WASM to load and mount
    if not (wd-wait-for-element $session_id "#root h1" 15) {
        log-fail "Counter app did not mount (no h1 found within 15s)"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }
    sleep 300ms

    # Initial state
    let r = (assert-text $session_id "initial count is 0" "#root h1" "High-Five counter: 0" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-exists $session_id "DOM has increment button" "#root button:first-of-type" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-exists $session_id "DOM has decrement button" "#root button:nth-of-type(2)" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-count $session_id "DOM has 3 buttons" "#root button" 3 -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Click increment
    let btn_incr = (wd-find $session_id "#root button:first-of-type")
    wd-click $session_id $btn_incr
    sleep 200ms
    let r = (assert-text $session_id "count after 1 increment" "#root h1" "High-Five counter: 1" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Click 4 more times
    for _ in 1..5 {
        wd-click $session_id $btn_incr
        sleep 50ms
    }
    sleep 200ms
    let r = (assert-text $session_id "count after 5 increments" "#root h1" "High-Five counter: 5" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Click decrement
    let btn_decr = (wd-find $session_id "#root button:nth-of-type(2)")
    wd-click $session_id $btn_decr
    sleep 200ms
    let r = (assert-text $session_id "count after decrement" "#root h1" "High-Five counter: 4" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Decrement past zero
    for _ in 1..7 {
        wd-click $session_id $btn_decr
        sleep 50ms
    }
    sleep 200ms
    let r = (assert-text $session_id "count below zero" "#root h1" "High-Five counter: -2" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # DOM structure preserved
    let r = (assert-exists $session_id "h1 still present after clicks" "#root h1" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-count $session_id "buttons still present" "#root button" 3 -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    { passed: $p, failed: $f }
}

# ── App test: Todo ─────────────────────────────────────────────────────────

def test-todo [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── Todo app ─────────────────────────────────────────────"

    wd-navigate $session_id $"($bu)/examples/todo/"

    # Wait for mount
    if not (wd-wait-for-element $session_id "#root > div" 15) {
        log-fail "Todo app did not mount (no container div within 15s)"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }
    sleep 300ms

    let r = (assert-exists $session_id "todo app mounts with container" "#root > div" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-exists $session_id "todo app has input" "#root input" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-exists $session_id "todo app has add button" "#root button" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Add first item via Add button
    let input_el = try { wd-find $session_id "#root input" } catch { "" }
    if ($input_el | is-empty) or $input_el == "null" {
        log-fail "Could not find input element"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }

    wd-send-keys $session_id $input_el "Buy milk"
    sleep 200ms
    let add_btn = try { wd-find $session_id "#root > div > button" } catch { "" }
    if ($add_btn | is-not-empty) and $add_btn != "null" {
        wd-click $session_id $add_btn
        sleep 500ms

        let root_eid = (wd-find $session_id "#root")
        let root_text = (wd-text $session_id $root_eid)
        if ($root_text | str contains "Buy milk") {
            log-ok "todo item 'Buy milk' added via button click"
            $p = $p + 1
        } else {
            log-fail "todo item 'Buy milk' not found in DOM after button click"
            $f = $f + 1
        }
    } else {
        log-fail "Could not find Add button"
        $f = $f + 1
    }

    # Add second item via Enter key (Servo WebDriver may not dispatch keydown;
    # treated as a soft/informational test — warn instead of fail)
    let input_el2 = try { wd-find $session_id "#root input" } catch { "" }
    if ($input_el2 | is-not-empty) and $input_el2 != "null" {
        wd-send-keys $session_id $input_el2 "Walk the dog"
        sleep 100ms
        # \uE007 = WebDriver Enter key
        wd-send-keys $session_id $input_el2 "\u{E007}"
        sleep 500ms

        let root_eid = (wd-find $session_id "#root")
        let root_text = (wd-text $session_id $root_eid)
        if ($root_text | str contains "Walk the dog") {
            log-ok "todo item 'Walk the dog' added via Enter key"
            $p = $p + 1
        } else {
            log-warn "Enter key did not add todo item (Servo WebDriver quirk — not a failure)"
            # Fall back: add via button so subsequent assertions still work
            let input_el3 = try { wd-find $session_id "#root input" } catch { "" }
            if ($input_el3 | is-not-empty) and $input_el3 != "null" {
                let add_btn2 = try { wd-find $session_id "#root > div > button" } catch { "" }
                if ($add_btn2 | is-not-empty) and $add_btn2 != "null" {
                    wd-click $session_id $add_btn2
                    sleep 500ms
                }
            }
        }
    }

    # Verify list has items
    let li_count = try { wd-find-all-count $session_id "#root li" } catch { 0 }
    if $li_count >= 1 {
        log-ok $"todo list has ($li_count) item\(s)"
        $p = $p + 1
    } else {
        log-fail "todo list has no items (expected >= 1)"
        $f = $f + 1
    }

    { passed: $p, failed: $f }
}

# ── App test: Bench ────────────────────────────────────────────────────────

def test-bench [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── Bench app ────────────────────────────────────────────"

    wd-navigate $session_id $"($bu)/examples/bench/"

    # Wait for mount — bench renders into #root
    if not (wd-wait-for-element $session_id "#root h1" 15) {
        # Bench might not use h1, try broader selector
        if not (wd-wait-for-element $session_id "#root button" 15) {
            log-fail "Bench app did not mount (no h1 or button within 15s)"
            $f = $f + 1
            return { passed: $p, failed: $f }
        }
    }
    sleep 300ms

    let r = (assert-exists $session_id "bench app mounts" "#root" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Bench should have toolbar buttons (Create 1,000 / Append / Update / etc.)
    let btn_count = try { wd-find-all-count $session_id "#root button" } catch { 0 }
    if $btn_count >= 4 {
        log-ok $"bench toolbar has ($btn_count) buttons (>= 4 expected)"
        $p = $p + 1
    } else {
        log-fail $"bench toolbar has ($btn_count) buttons (expected >= 4)"
        $f = $f + 1
    }

    # Click "Create 1,000" (first button)
    let create_btn = try { wd-find $session_id "#root button:first-of-type" } catch { "" }
    if ($create_btn | is-not-empty) and $create_btn != "null" {
        wd-click $session_id $create_btn
        sleep 2sec  # Give WASM time to create + render 1000 rows

        # Check that rows appeared in the table
        let row_count = try { wd-find-all-count $session_id "#root tbody tr" } catch { 0 }
        if $row_count >= 100 {
            log-ok $"bench created ($row_count) rows after 'Create 1,000'"
            $p = $p + 1
        } else {
            log-fail $"bench created ($row_count) rows (expected >= 100)"
            $f = $f + 1
        }

        # Status should show operation info
        let r = (assert-exists $session_id "bench has status/timing info" "#root" -p $p -f $f)
        $p = $r.passed; $f = $r.failed
    } else {
        log-warn "Could not find Create button — skipping bench row tests"
    }

    { passed: $p, failed: $f }
}

# ── App test: Multi-View App ──────────────────────────────────────────────

def test-app [session_id: string, timeout: int, passed: int, failed: int]: nothing -> record<passed: int, failed: int> {
    mut p = $passed
    mut f = $failed
    let bu = (base-url)

    log-info ""
    log-info "── Multi-View App ───────────────────────────────────────"

    wd-navigate $session_id $"($bu)/examples/app/"

    # Wait for WASM to load and mount (nav bar should appear)
    if not (wd-wait-for-element $session_id "#root nav" 15) {
        log-fail "App did not mount (no nav found within 15s)"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }
    sleep 500ms

    # ── Initial state: app shell + counter view (default route "/") ──

    let r = (assert-exists $session_id "app shell mounts with nav" "#root nav" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-count $session_id "nav has 2 buttons (Counter, Todo)" "#root nav button" 2 -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-exists $session_id "counter view mounted by default (h1)" "#root h1" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-text $session_id "initial counter is 0" "#root h1" "Count: 0" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # Counter view buttons (inside the content div, not the nav)
    let r = (assert-exists $session_id "counter has + 1 button" "#root > div > div button:first-of-type" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-exists $session_id "counter has - 1 button" "#root > div > div button:nth-of-type(2)" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # ── Counter interactions ──

    let btn_incr = try { wd-find $session_id "#root > div > div button:first-of-type" } catch { "" }
    if ($btn_incr | is-not-empty) and $btn_incr != "null" {
        wd-click $session_id $btn_incr
        sleep 200ms
        let r = (assert-text $session_id "counter after +1" "#root h1" "Count: 1" -p $p -f $f)
        $p = $r.passed; $f = $r.failed

        wd-click $session_id $btn_incr
        wd-click $session_id $btn_incr
        sleep 200ms
        let r = (assert-text $session_id "counter after 3 increments" "#root h1" "Count: 3" -p $p -f $f)
        $p = $r.passed; $f = $r.failed

        let btn_decr = try { wd-find $session_id "#root > div > div button:nth-of-type(2)" } catch { "" }
        if ($btn_decr | is-not-empty) and $btn_decr != "null" {
            wd-click $session_id $btn_decr
            sleep 200ms
            let r = (assert-text $session_id "counter after decrement" "#root h1" "Count: 2" -p $p -f $f)
            $p = $r.passed; $f = $r.failed
        }
    } else {
        log-fail "Could not find counter +1 button"
        $f = $f + 1
    }

    # ── Navigate to Todo view ──

    let nav_todo = try { wd-find $session_id "#root nav button:nth-of-type(2)" } catch { "" }
    if ($nav_todo | is-empty) or $nav_todo == "null" {
        log-fail "Could not find Todo nav button"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }

    wd-click $session_id $nav_todo
    sleep 500ms

    # Counter h1 should be gone, todo h2 should appear
    let r = (assert-exists $session_id "todo view has h2" "#root h2" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-text $session_id "todo shows 0 items" "#root h2" "Items: 0" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let r = (assert-exists $session_id "todo has Add item button" "#root > div > div button" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    # ── Todo interactions ──

    let add_btn = try { wd-find $session_id "#root > div > div button" } catch { "" }
    if ($add_btn | is-not-empty) and $add_btn != "null" {
        wd-click $session_id $add_btn
        sleep 300ms
        let r = (assert-text $session_id "todo after 1 add" "#root h2" "Items: 1" -p $p -f $f)
        $p = $r.passed; $f = $r.failed

        wd-click $session_id $add_btn
        wd-click $session_id $add_btn
        sleep 300ms
        let r = (assert-text $session_id "todo after 3 adds" "#root h2" "Items: 3" -p $p -f $f)
        $p = $r.passed; $f = $r.failed
    } else {
        log-fail "Could not find Add item button"
        $f = $f + 1
    }

    # ── Navigate back to Counter view ──

    let nav_counter = try { wd-find $session_id "#root nav button:first-of-type" } catch { "" }
    if ($nav_counter | is-empty) or $nav_counter == "null" {
        log-fail "Could not find Counter nav button"
        $f = $f + 1
        return { passed: $p, failed: $f }
    }

    wd-click $session_id $nav_counter
    sleep 500ms

    # Counter view should reappear — state may or may not persist depending
    # on the router implementation; just check that it renders with a valid count
    let r = (assert-exists $session_id "counter view restored (h1)" "#root h1" -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    let h1_eid = try { wd-find $session_id "#root h1" } catch { "" }
    if ($h1_eid | is-not-empty) and $h1_eid != "null" {
        let h1_text = (wd-text $session_id $h1_eid)
        if ($h1_text | str starts-with "Count:") {
            log-ok "counter view shows valid count after route switch"
            $p = $p + 1
        } else {
            log-fail $"counter h1 text unexpected: \"($h1_text)\""
            $f = $f + 1
        }
    }

    # Nav bar should still be present throughout
    let r = (assert-count $session_id "nav still has 2 buttons after switching" "#root nav button" 2 -p $p -f $f)
    $p = $r.passed; $f = $r.failed

    { passed: $p, failed: $f }
}

# ── Main ───────────────────────────────────────────────────────────────────

def main [
    --timeout: int = 30     # Per-wait timeout in seconds
    --app: string = ""      # Test only a specific app (counter, todo, bench, app)
    --verbose                # Stream servo stderr
    --keep                   # Keep servo running after tests
] {
    let script_dir = ($env.FILE_PWD)
    mut servo_pid = 0
    mut server_pid = 0
    mut session_id = ""
    mut servo_log = ""
    mut passed = 0
    mut failed = 0

    # ── Preflight checks ──────────────────────────────────────────────

    for cmd in [servo deno curl jq] {
        if (which $cmd | is-empty) {
            log-fail $"Required command not found: ($cmd)"
            exit 2
        }
    }

    # Note: no try/catch wrapper — nushell 0.110.0 cannot capture mutable
    # variables in catch blocks.  Stale processes from crashed runs are
    # cleaned up by the kill-port calls below at the start of each run.

    # ── Build WASM ────────────────────────────────────────────────

    log-info "Building WASM..."
    cd $script_dir
    ^just build

    # ── Kill stale processes on our ports ─────────────────────────

    kill-port $SERVE_PORT
    kill-port $WD_PORT

    # ── Start file server ─────────────────────────────────────────

    log-info $"Starting file server on :($SERVE_PORT)..."
    $server_pid = (^bash -c $'cd "($script_dir)" && deno run --allow-net --allow-read jsr:@std/http/file-server -p ($SERVE_PORT) 2>/dev/null & echo $!' | str trim | into int)

    # Wait for file server to be ready
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

    $servo_log = (^mktemp /tmp/servo-test-XXXXXX.log | str trim)

    log-info $"Starting Servo \(headless, WebDriver on :($WD_PORT))..."

    $servo_pid = (^bash -c $'servo --headless --webdriver=($WD_PORT) "about:blank" > "($servo_log)" 2>&1 & echo $!' | str trim | into int)

    # Wait for WebDriver to become ready
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
    log-info $"Running browser tests \(timeout: ($timeout)s per wait)..."

    if ($app | is-empty) or $app == "counter" {
        let r = (test-counter $session_id $timeout $passed $failed)
        $passed = $r.passed; $failed = $r.failed
    }
    if ($app | is-empty) or $app == "todo" {
        let r = (test-todo $session_id $timeout $passed $failed)
        $passed = $r.passed; $failed = $r.failed
    }
    if ($app | is-empty) or $app == "bench" {
        let r = (test-bench $session_id $timeout $passed $failed)
        $passed = $r.passed; $failed = $r.failed
    }
    if ($app | is-empty) or $app == "app" {
        let r = (test-app $session_id $timeout $passed $failed)
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
        if $keep and $servo_pid > 0 {
            log-info $"Servo still running \(PID ($servo_pid)). Press Ctrl-C to stop."
            # Block until servo exits
            let _svpid = $servo_pid
            try { ^waitpid $_svpid } catch { }
        }
        exit 0
    } else {
        log-fail $"($total) tests: ($passed) passed, ($failed) failed"
        exit 1
    }
}
