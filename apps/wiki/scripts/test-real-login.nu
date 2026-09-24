#!/usr/bin/env nu

# test-real-login.nu: one REAL sign-in, all of it on this machine.
#
# The browser suite walks in with a code a dev AppView minted. What that cannot
# show is a provider: the pushed request with DPoP and PKCE, the provider's own
# sign-in and consent pages, the token exchange, the profile read, a post
# written into the account's repo. This runs the real `appview` against a real
# PDS (nixpkgs' `bluesky-pds`), which registers its made-up account with a
# stand-in `did:plc` directory (`crates/fake-plc`) since the public one is no
# place for it, and drives the frontend through all of it in headless Firefox.
#
# The account is typed in by its DID: a handle under `.test` resolves nowhere.
# A PDS on plain http is never believed about an address (`trusts_email_of`), so
# taking an old account over by address is not part of this; the cutover
# rehearsal covers it.
#
# Usage: nu scripts/test-real-login.nu [--keep]
# Exit codes: 0 all passed · 1 a check failed · 2 setup failed

const WD_PORT = 7135
const SERVE_PORT = 8135
const API_PORT = 8136
const PLC_PORT = 2582
const PDS_PORT = 2583

def wd [] { $"http://127.0.0.1:($WD_PORT)" }
def app [] { $"http://127.0.0.1:($SERVE_PORT)" }
def api [] { $"http://127.0.0.1:($API_PORT)/xrpc/wiki.radikal" }
def provider [] { $"http://localhost:($PDS_PORT)" }

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset)  ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset)  ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset)  ($msg | str join ' ')" }

def wd-post [path: string, body: any] {
    try { http post --content-type application/json $"(wd)($path)" ($body | to json) } catch { null }
}

def js [sid: string, script: string, args: list<any> = []] {
    let r = (wd-post $"/session/($sid)/execute/sync" { script: $script, args: $args })
    if $r == null { null } else { $r | get -o value }
}

def here [sid: string]: nothing -> string {
    try { http get $"(wd)/session/($sid)/url" | get value } catch { "" }
}

# Poll `probe` (JS answering true) for up to `secs`.
def wait-for [sid: string, probe: string, secs: int]: nothing -> bool {
    mut waited = 0
    while $waited < ($secs * 2) {
        if (js $sid $probe) == true { return true }
        sleep 500ms
        $waited = $waited + 1
    }
    false
}

# A framework's input keeps its own copy of the value, and hears only the
# native setter followed by an input event.
const TYPE = 'const i = document.querySelector(arguments[0]); if (!i) return false; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(i, arguments[1]); i.dispatchEvent(new Event("input", { bubbles: true })); return true;'
const PRESS = 'const b = [...document.querySelectorAll("button[type=submit]")].find(b => b.innerText.trim() === arguments[0]); if (!b) return false; b.click(); return true;'

def kill-port [port: int] {
    try { ^fuser -k $"($port)/tcp" | complete | ignore } catch { }
}

# Start `command` detached, its output in `log`, and return its pid.
def start [command: string, log: string]: nothing -> int {
    ^bash -c $'($command) > "($log)" 2>&1 < /dev/null & echo $!' | str trim | into int
}

def cleanup [sid: string, pids: list<int>] {
    if ($sid | is-not-empty) { try { ^curl -s -X DELETE $"(wd)/session/($sid)" | ignore } catch { } }
    for pid in $pids { try { ^kill $pid | complete | ignore } catch { } }
    # Launched through wrappers whose pid is not the listener's.
    for port in [$WD_PORT $SERVE_PORT $API_PORT $PLC_PORT $PDS_PORT] { kill-port $port }
}

def main [
    --keep  # Leave everything running after
] {
    let proj = ($env.FILE_PWD | path dirname)
    cd $proj
    let tmp = (^mktemp -d | str trim)
    for cmd in [dx curl cargo fuser] {
        if (which $cmd | is-empty) { log-fail $"Required command not found: ($cmd)"; exit 2 }
    }
    for port in [$WD_PORT $SERVE_PORT $API_PORT $PLC_PORT $PDS_PORT] { kill-port $port }

    log-info "Building the directory stand-in and the AppView..."
    let built = (do -i { ^cargo build --quiet -p fake-plc -p appview --manifest-path crates/Cargo.toml } | complete)
    if $built.exit_code != 0 { log-fail "the build failed"; print -e $built.stderr; exit 2 }
    let pds_bin = if (which pds | where type == "external" | is-not-empty) { "pds" } else {
        log-info "Fetching a PDS from nixpkgs..."
        $"(^nix build nixpkgs#bluesky-pds --no-link --print-out-paths | str trim)/bin/pds"
    }

    let plc_pid = (start $"crates/target/debug/fake-plc --port ($PLC_PORT)" $"($tmp)/plc.log")
    mkdir $"($tmp)/pds" $"($tmp)/blocks"
    let pds_env = ([
        "PDS_HOSTNAME=localhost" $"PDS_PORT=($PDS_PORT)" "PDS_DEV_MODE=true" "NODE_ENV=development"
        $"PDS_DATA_DIRECTORY=($tmp)/pds" $"PDS_BLOBSTORE_DISK_LOCATION=($tmp)/blocks"
        $"PDS_DID_PLC_URL=http://127.0.0.1:($PLC_PORT)"
        $"PDS_JWT_SECRET=(random chars --length 32)" $"PDS_ADMIN_PASSWORD=(random chars --length 24)"
        $"PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX=(random binary 32 | encode hex | str lowercase)"
        "PDS_SERVICE_HANDLE_DOMAINS=.test" "PDS_INVITE_REQUIRED=false" "LOG_ENABLED=true"
    ] | str join " ")
    let pds_pid = (start $"($pds_env) ($pds_bin)" $"($tmp)/pds.log")
    # Nothing to hear on the firehose here, and no reason to reach for the real one.
    let api_pid = (start $"PORT=($API_PORT) APPVIEW_DB=($tmp)/wiki.db APPVIEW_PLC_URL=http://127.0.0.1:($PLC_PORT) APPVIEW_FRONTEND_ORIGINS=(app) JETSTREAM_URL=ws://127.0.0.1:9 crates/target/debug/appview" $"($tmp)/appview.log")

    mut up = false
    mut waited = 0
    while $waited < 60 and not $up {
        $up = ((do -i { ^curl -s -m 2 -o /dev/null -w "%{http_code}" $"(provider)/xrpc/_health" } | complete).stdout | str trim) == "200"
        if not $up { sleep 1sec }
        $waited = $waited + 1
    }
    if not $up { log-fail $"the PDS did not come up; see ($tmp)/pds.log"; cleanup "" [$plc_pid $pds_pid $api_pid]; exit 2 }

    let password = (random chars --length 24)
    let account = (http post --content-type application/json $"(provider)/xrpc/com.atproto.server.createAccount" ({handle: "alice.test", email: "alice@wiki.test", password: $password} | to json))
    let did = $account.did
    log-info $"The PDS made alice.test, ($did), and registered her with the directory"

    log-info $"Serving the AppView build on :($SERVE_PORT) \(the first build takes a few minutes)..."
    let dx = (^bash -c "which -a dx | grep dioxus | head -1" | str trim)
    let serve_pid = (start $"WIKI_APPVIEW_URL=http://127.0.0.1:($API_PORT) ($dx) serve --features appview --interactive false --open false --port ($SERVE_PORT)" $"($tmp)/dx.log")
    let gecko = if (which geckodriver | is-not-empty) and (which firefox | is-not-empty) {
        $"geckodriver --port ($WD_PORT)"
    } else {
        $"nix shell nixpkgs#geckodriver nixpkgs#firefox --command geckodriver --port ($WD_PORT)"
    }
    let wd_pid = (start $gecko $"($tmp)/gecko.log")
    let pids = [$plc_pid $pds_pid $api_pid $serve_pid $wd_pid]

    mut built_ok = false
    mut waited = 0
    while $waited < 900 {
        let log = (try { open --raw $"($tmp)/dx.log" } catch { "" })
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
    mut passed = 0
    mut failed = 0

    # The wiki's own form, which hands the browser to the account's provider.
    wd-post $"/session/($sid)/url" { url: $"(app)/user/login" } | ignore
    wait-for $sid 'return !!document.getElementById("auth-handle")' 120 | ignore
    js $sid $TYPE ["#auth-handle" $did] | ignore
    js $sid 'document.querySelector("form.auth-form button[type=submit]").click(); return true;' | ignore
    let at_the_provider = (wait-for $sid 'return location.host === "localhost:2583" && !!document.querySelector("input[name=password]")' 60)
    let named = (js $sid 'return (document.querySelector("input[name=username]") || {}).value || ""')
    if $at_the_provider and $named == "alice.test" {
        $passed = $passed + 1
        log-ok "the wiki's form hands the browser to the account's own provider, which knows who is asking"
    } else {
        $failed = $failed + 1
        log-fail $"not at the provider's sign-in page: there: ($at_the_provider), for: ($named), at: (here $sid | str substring 0..80)"
    }

    # Her password goes to her provider and nowhere else; then she approves.
    js $sid $TYPE ["input[name=password]" $password] | ignore
    js $sid $PRESS ["Sign in"] | ignore
    let asked = (wait-for $sid 'return [...document.querySelectorAll("button[type=submit]")].some(b => b.innerText.trim() === "Authorize")' 60)
    let scopes = (js $sid 'return document.body.innerText')
    js $sid $PRESS ["Authorize"] | ignore
    let back = (wait-for $sid 'const s = JSON.parse(localStorage.getItem("wiki_session") || "null"); return location.host === "127.0.0.1:8135" && !!(s && s.access_token) && !document.getElementById("auth-handle");' 60)
    let shown = (wait-for $sid 'return ((document.querySelector(".drawer-account-trigger") || {}).innerText || "").includes("alice.test")' 30)
    if $asked and $back and $shown {
        $passed = $passed + 1
        log-ok "signing in there and approving comes back signed in, off the form, under her handle"
    } else {
        $failed = $failed + 1
        log-fail $"the provider asked for approval: ($asked), back and signed in: ($back), the account menu names her: ($shown), at: (here $sid | str substring 0..80)"
    }
    if ($scopes | default "" | str contains "email address") {
        $passed = $passed + 1
        log-ok "the provider told her the wiki asks to read her address"
    } else {
        $failed = $failed + 1
        log-fail "the consent page did not name the address scope"
    }

    let token = (js $sid 'const s = JSON.parse(localStorage.getItem("wiki_session") || "null"); return s ? s.access_token : ""' | default "")
    let me = (try { http get --headers {authorization: $"Bearer ($token)"} $"(api).getSession" } catch { {} })
    if ($me | get -o did) == $did and ($me | get -o handle) == "alice.test" {
        $passed = $passed + 1
        log-ok "the AppView knows her by what her PDS says of her"
    } else {
        $failed = $failed + 1
        log-fail $"getSession said ($me | to json -r)"
    }

    # With the grant her sign-in left: DPoP-bound, and the PDS's to refuse.
    let text = "A rehearsal post from the wiki"
    let shared = (try { http post --content-type application/json --headers {authorization: $"Bearer ($token)"} $"(api).shareToBluesky" ({text: $text} | to json) } catch { {} })
    let kept = (try { http get $"(provider)/xrpc/com.atproto.repo.listRecords?repo=($did)&collection=app.bsky.feed.post" | get records | get value.text } catch { [] })
    if ($shared | get -o uri | default "" | str starts-with $"at://($did)/") and ($text in $kept) {
        $passed = $passed + 1
        log-ok "a post is written into her repo, and her PDS has it"
    } else {
        $failed = $failed + 1
        log-fail $"sharing said ($shared | to json -r), and the PDS holds ($kept | to json -r)"
    }

    js $sid 'const t = document.querySelector(".drawer-account-trigger"); if (t) t.click(); return true;' | ignore
    wait-for $sid 'const out = [...document.querySelectorAll("button.list-item")].find(b => b.innerText.replace(/\s+/g, " ").trim() === "logout Log out"); if (!out) return false; out.click(); return true;' 15 | ignore
    sleep 3sec
    let still = (^curl -s -o /dev/null -w "%{http_code}" -H $"authorization: Bearer ($token)" $"(api).getSession" | str trim)
    if $still == "401" {
        $passed = $passed + 1
        log-ok "signing out ends the session at the AppView"
    } else {
        $failed = $failed + 1
        log-fail $"after signing out the session answered ($still)"
    }

    let errors = (try { open --raw $"($tmp)/gecko.log" | lines | where { |l| $l =~ '%cERROR%c|panicked at' } } catch { [] })
    if ($errors | is-empty) {
        $passed = $passed + 1
        log-ok "the app logged no error"
    } else {
        $failed = $failed + 1
        log-fail $"the app logged ($errors | length) errors, the first: ($errors | first | str substring 0..300)"
    }

    if $keep {
        log-info $"Left running. Logs: ($tmp). The account's password is in nobody's log."
    } else {
        cleanup $sid $pids
        if $failed == 0 { rm -rf $tmp } else { log-info $"Logs: ($tmp)" }
    }
    log-info $"($passed) passed, ($failed) failed."
    if $failed > 0 { exit 1 }
}
