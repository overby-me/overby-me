#!/usr/bin/env nu

# test-browser.nu — Load the RadikalWiki Dioxus app in a headless browser and
#                   verify DOM state via W3C WebDriver (nushell + curl + jq).
#
# The app is a single Dioxus/WASM SPA served by `dx serve` (debug build). Two
# WebDriver backends: headless Servo (default) or real Firefox via geckodriver
# (--firefox). Prefer --firefox: Servo masks client-side routing / rendering bugs
# that real browsers hit (it hid the whole navigation-stale-view bug).
#
# Usage:
#   nu test-browser.nu                     # Unauthenticated smoke tests (Servo)
#   nu test-browser.nu --firefox           # Drive real Firefox (geckodriver)
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

    # ── In-context navigation (drawer node tree + app rail) ──────────────
    # Click the first context; the app should route into it, render a node
    # view, switch the drawer from the home list to the MenuList tree, and
    # reveal the app rail.
    let first_ctx = (try { wd-find $session_id ".drawer .avatar.secondary" } catch { "" })
    if ($first_ctx | is-empty) or $first_ctx == "null" {
        log-warn "no context to open — skipping in-context checks"
        return { passed: $p, failed: $fl }
    }
    # Click the list-item that carries the onclick handler (the avatar is just a
    # child span), dispatching a DOM click the Dioxus delegated listener sees.
    wd-execute $session_id 'var e=document.querySelector(".drawer .avatar.secondary"); if(e){e.closest(".list-item").click()} return e?"clicked":"none"' | ignore
    mut navigated = false
    for _ in 1..($timeout) {
        let path = (wd-execute $session_id 'return location.pathname')
        if ($path != null) and ($path != "/") { $navigated = true; break }
        sleep 500ms
    }
    if not $navigated {
        log-fail "clicking a context did not navigate"
        return { passed: $p, failed: ($fl + 1) }
    }
    log-ok "navigated into a context"; $p = $p + 1
    sleep 2sec

    let r = (assert-exists $session_id "context view renders a card" "#main .card" -p $p -f $fl); $p = $r.passed; $fl = $r.failed
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
    # Go back to the context, then click the Vote rail item and confirm the URL
    # gains ?app=vote and the vote view renders. Click via JS on the anchor
    # (WebDriver's click can land on an inner span the router doesn't intercept).
    let ctx_path = (wd-execute $session_id 'return "/"+location.pathname.split("/")[1]')
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
        } else {
            log-warn "no vote rail item — skipping app-switch check"
        }
    } else {
        log-warn "app rail not found — skipping app-switch check"
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

    { passed: $p, failed: $fl }
}

# ── Main ────────────────────────────────────────────────────────────────────

def main [
    --timeout: int = 30
    --verbose
    --keep
    --firefox   # Drive real Firefox (via geckodriver) instead of Servo. Firefox
                # catches client-side routing / rendering bugs Servo masks.
] {
    let proj = $env.FILE_PWD
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
