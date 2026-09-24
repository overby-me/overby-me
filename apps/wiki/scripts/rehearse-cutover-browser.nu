#!/usr/bin/env nu
# The last of a cutover rehearsal: the frontend built on the AppView, in headless
# Firefox, walking REAL pages as a member who has taken their old account over.
# Made-up pages are what the browser suite draws; this is for what only real
# ones have: a page body in a shape no fixture thought of, a folder of hundreds.
#
# Run `scripts/rehearse-cutover.nu <dir>` first: this serves the datastore it
# left in <dir>/returns. It prints kinds, ids and verdicts, and never a title.
#
# Usage: nu scripts/rehearse-cutover-browser.nu <dir> [--each 6]
# Exit codes: 0 every page drew · 1 one did not, or the app logged an error · 2 setup failed

const WD_PORT = 7135
const SERVE_PORT = 8135
const API_PORT = 8136

def wd [] { $"http://127.0.0.1:($WD_PORT)" }
def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset)  ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset)  ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset)  ($msg | str join ' ')" }

def wd-post [path: string, body: any] {
    try { http post --content-type application/json $"(wd)($path)" ($body | to json) } catch { null }
}

def js [sid: string, script: string] {
    let r = (wd-post $"/session/($sid)/execute/sync" { script: $script, args: [] })
    if $r == null { null } else { $r | get -o value }
}

def kill-port [port: int] {
    try { ^fuser -k $"($port)/tcp" | complete | ignore } catch { }
}

# Start `command` detached, its output in `log`, and return its pid.
def start [command: string, log: string]: nothing -> int {
    ^bash -c $'($command) > "($log)" 2>&1 & echo $!' | str trim | into int
}

def cleanup [sid: string, pids: list<int>] {
    if ($sid | is-not-empty) { try { ^curl -s -X DELETE $"(wd)/session/($sid)" | ignore } catch { } }
    for pid in $pids { try { ^kill $pid | complete | ignore } catch { } }
    # Both are launched through wrappers whose pid is not the listener's.
    for port in [$WD_PORT $SERVE_PORT $API_PORT] { kill-port $port }
}

# What the dev AppView said of itself, once it has: its one line of JSON, among
# whatever else it logged into the same file.
def hello [log: string] {
    try { open --raw $log | lines | where { |l| $l | str starts-with "{" } | last | from json } catch { null }
}

# Whether the page says `title` within `secs`, spaces and case aside.
def draws [sid: string, title: string, secs: int]: nothing -> bool {
    let probe = ("const flat = s => s.replace(/\\s+/g, ' ').trim().toLowerCase(); return flat(document.body.innerText).includes(flat(" + ($title | to json -r) + "));")
    mut waited = 0
    while $waited < ($secs * 2) {
        if (js $sid $probe) == true { return true }
        sleep 500ms
        $waited = $waited + 1
    }
    false
}

def main [dir: path, --each: int = 6] {
    let dir = ($dir | path expand)
    let db = ($dir | path join "returns" "wiki.db")
    if not ($db | path exists) { log-fail $"no ($db): run scripts/rehearse-cutover.nu first"; exit 2 }
    let proj = ($env.FILE_PWD | path dirname)
    cd $proj
    let logs = ($dir | path join "browser")
    rm -rf $logs
    mkdir $logs
    for port in [$WD_PORT $SERVE_PORT $API_PORT] { kill-port $port }

    let people = (0..39 | each { |i| $"did:plc:returned($i)" } | str join " ")
    let blobs = ($dir | path join "stage" "blobs")
    let api_pid = (start $"APPVIEW_BLOB_DIR=($blobs) APPVIEW_FRONTEND_ORIGINS=http://127.0.0.1:($SERVE_PORT) crates/target/release/appview-dev --port ($API_PORT) --db ($db) ($people)" $"($logs)/appview.json")
    mut waited = 0
    while $waited < 300 and (hello $"($logs)/appview.json") == null {
        sleep 1sec
        $waited = $waited + 1
    }
    let said = (hello $"($logs)/appview.json")
    if $said == null { log-fail "the dev AppView did not start"; cleanup "" [$api_pid]; exit 2 }

    # The returned account with the most seats has the most to show.
    let api = $"($said.url)/xrpc/wiki.radikal"
    let seated = ($said.sessions | transpose did token | each { |p|
        let mine = (http get --headers {authorization: $"Bearer ($p.token)"} $"($api).listContexts?scope=mine")
        {did: $p.did, token: $p.token, contexts: ($mine.contexts | get id)}
    } | sort-by { |p| $p.contexts | length } | last)
    log-info $"Walking as ($seated.did), who has a seat in ($seated.contexts | length) contexts"

    log-info $"Serving the AppView build on :($SERVE_PORT) \(the first build takes a few minutes)..."
    let dx = (^bash -c "which -a dx | grep dioxus | head -1" | str trim)
    let serve_pid = (start $"WIKI_APPVIEW_URL=http://127.0.0.1:($API_PORT) ($dx) serve --features appview --interactive false --open false --port ($SERVE_PORT)" $"($logs)/dx.log")
    let gecko = if (which geckodriver | is-not-empty) and (which firefox | is-not-empty) {
        $"geckodriver --port ($WD_PORT)"
    } else {
        $"nix shell nixpkgs#geckodriver nixpkgs#firefox --command geckodriver --port ($WD_PORT)"
    }
    let wd_pid = (start $gecko $"($logs)/gecko.log")
    let pids = [$api_pid $serve_pid $wd_pid]

    mut built_ok = false
    mut waited = 0
    while $waited < 900 {
        let log = (try { open --raw $"($logs)/dx.log" } catch { "" })
        if ($log | str contains "Build completed") { $built_ok = true; break }
        if (do -i { ^kill -0 $serve_pid } | complete).exit_code != 0 { break }
        sleep 2sec
        $waited = $waited + 2
    }
    if not $built_ok { log-fail "dx serve did not finish its first build"; cleanup "" $pids; exit 2 }

    let caps = { capabilities: { alwaysMatch: { "moz:firefoxOptions": {
        args: ["-headless"], prefs: { "devtools.console.stdout.content": true }
    } } } }
    mut sid = ""
    mut tries = 0
    while $tries < 60 and ($sid | is-empty) {
        let made = (wd-post "/session" $caps)
        $sid = (if $made == null { "" } else { $made | get -o value.sessionId | default "" })
        if ($sid | is-empty) { sleep 1sec }
        $tries = $tries + 1
    }
    if ($sid | is-empty) { log-fail "no WebDriver session"; cleanup "" $pids; exit 2 }
    let sid = $sid
    wd-post $"/session/($sid)/window/rect" { width: 1280, height: 900 } | ignore

    # What `finish_sign_in` stores once a provider has sent the browser back.
    wd-post $"/session/($sid)/url" { url: $"http://127.0.0.1:($SERVE_PORT)/" } | ignore
    sleep 3sec
    let stored = ({
        user: { id: $seated.did, email: "", display_name: "", avatar_url: "" },
        access_token: $seated.token, refresh_token: $seated.token, node_id: null,
        access_token_expires_at: 4102444800000.0
    } | to json -r)
    js $sid ("localStorage.setItem('wiki_session', " + ($stored | to json -r) + "); return 1") | ignore
    # The first load compiles the whole debug build. Waited for here, it is not
    # held against whichever page happens to be first.
    wd-post $"/session/($sid)/url" { url: $"http://127.0.0.1:($SERVE_PORT)/" } | ignore
    mut ready = 0
    while $ready < 240 and (js $sid 'return !!document.querySelector(".drawer-account-trigger")') != true {
        sleep 500ms
        $ready = $ready + 1
    }

    # Some of every kind, the longest bodies first: they are the likeliest to
    # hold a shape nothing else has.
    let ex = (open ($dir | path join "extraction.json"))
    let mine = $seated.contexts
    let pages = ($ex.documents
        | where { |d| $d.context_id in $mine and ($d.deleted_at? | is-empty) and ($d.title | str trim | is-not-empty) }
        | each { |d| {kind: $d.kind, id: $d.id, path: $d.path, title: $d.title, weight: ($d.content? | to json -r | str length)} }
        | group-by kind | values
        | each { |of_a_kind| $of_a_kind | sort-by weight --reverse | first $each }
        | flatten)
    let places = ($ex.contexts
        | where { |c| $c.id in $mine and ($c.name | str trim | is-not-empty) }
        | each { |c| {kind: $"context/($c.kind)", id: $c.id, path: $c.path, title: $c.name, weight: 0} }
        | first $each)
    let walk = ($places | append $pages)
    log-info $"($walk | length) pages to walk"

    mut failed = 0
    for page in $walk {
        let before = (try { open --raw $"($logs)/gecko.log" | lines | length } catch { 0 })
        let segments = ($page.path | split row "/" | each { |s| $s | url encode } | str join "/")
        let url = $"http://127.0.0.1:($SERVE_PORT)/($segments)"
        wd-post $"/session/($sid)/url" { url: $url } | ignore
        # Once more before it counts: slow is not the same as cannot.
        let drew = if (draws $sid $page.title 25) { true } else {
            wd-post $"/session/($sid)/url" { url: $url } | ignore
            draws $sid $page.title 25
        }
        let heard = (try { open --raw $"($logs)/gecko.log" | lines | skip $before | where { |l| $l =~ '%cERROR%c|panicked at|RuntimeError' } } catch { [] })
        if $drew and ($heard | is-empty) {
            log-ok $"($page.kind) ($page.id)"
        } else {
            $failed = $failed + 1
            log-fail $"($page.kind) ($page.id): drew its title: ($drew), errors logged: ($heard | length)"
            $heard | first 3 | each { |l| print -e $"         ($l | str substring 0..300)" } | ignore
        }
    }
    cleanup $sid $pids
    if $failed == 0 {
        log-info $"All ($walk | length) real pages drew, and the app logged no error."
    } else {
        log-info $"($failed) of ($walk | length) did not. Logs: ($logs)"
        exit 1
    }
}
