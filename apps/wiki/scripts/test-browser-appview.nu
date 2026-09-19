#!/usr/bin/env nu

# test-browser-appview.nu: the frontend built on the AppView (`--features
# appview`), driven in headless Firefox against a dev AppView it starts itself.
#
# `test-browser.nu` is the suite for what ships, and needs the interim's
# backend and an account on it. This one needs nothing outside this machine:
# `crates/appview-dev` is an empty in-memory wiki that hands out sessions, so a
# run seeds its own group, signs in by storing a session, and checks that the
# screens draw it, that writing through them lands, and that a change made
# elsewhere arrives without a reload.
#
# Usage:
#   nu scripts/test-browser-appview.nu            # build, serve, drive, clean up
#   nu scripts/test-browser-appview.nu --keep     # leave everything running after
#   nu scripts/test-browser-appview.nu --shots    # also save a PNG per screen
#
# Exit codes: 0 all passed · 1 a check failed · 2 setup failed

const WD_PORT = 7135
const SERVE_PORT = 8135
const API_PORT = 8136

def wd [] { $"http://127.0.0.1:($WD_PORT)" }
def app [] { $"http://127.0.0.1:($SERVE_PORT)" }
def api [] { $"http://127.0.0.1:($API_PORT)/xrpc/com.example.wiki" }

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset)  ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset)  ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset)  ($msg | str join ' ')" }

def wd-post [path: string, body: any] {
    try { ^curl -s -H "Content-Type: application/json" -d ($body | to json -r) $"(wd)($path)" | from json } catch { null }
}

# Run JavaScript in the page and hand back what it returns.
def js [sid: string, script: string] {
    let answer = (wd-post $"/session/($sid)/execute/sync" { script: $script, args: [] })
    if $answer == null { null } else { $answer | get -o value }
}

def shot [sid: string, out: string] {
    try { ^curl -s $"(wd)/session/($sid)/screenshot" | from json | get value | decode base64 | save -f $out } catch { }
}

def go [sid: string, path: string] {
    wd-post $"/session/($sid)/url" { url: $"(app)($path)" } | ignore
}

# Poll until the page's text contains `needle`, up to `secs`.
def wait-for-text [sid: string, needle: string, secs: int]: nothing -> bool {
    let probe = $"return document.body.innerText.includes\(($needle | to json -r)\)"
    mut waited = 0
    while $waited < ($secs * 2) {
        if (js $sid $probe) == true { return true }
        sleep 500ms
        $waited = $waited + 1
    }
    false
}

def xrpc [token: string, method: string, body: any] {
    ^curl -s -X POST -H $"authorization: Bearer ($token)" -H "content-type: application/json" -d ($body | to json -r) $"(api).($method)" | from json
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

# One of everything a group's screens show. Returns the ids.
def seed [owner: string, member: string] {
    let group = (xrpc $owner createContext { parent_id: "home", kind: "group", name: "Hovedbestyrelsen" } | get id)
    let folder = (xrpc $owner createDocument { context_id: $group, parent_id: $group, kind: "folder", title: "Bilag" } | get id)
    let page = (xrpc $owner createDocument {
        context_id: $group, parent_id: $folder, kind: "document", title: "Dagsorden", mutable: false,
        content: [{ type: "paragraph", children: [{ text: "Valg af dirigent og referent" }] }]
    } | get id)
    xrpc $owner postComment { on_id: $page, text: "Enig i dagsordenen" } | ignore
    xrpc $owner inviteMembers { context_id: $group, invites: [{ name: "Medlem", did: "did:plc:member" }] } | ignore
    let invitation = (^curl -s -H $"authorization: Bearer ($member)" $"(api).listInvitations" | from json | get invitations | first | get id)
    xrpc $member acceptInvitation { id: $invitation } | ignore
    let motions = (xrpc $owner createDocument { context_id: $group, parent_id: $group, kind: "folder", title: "Forslag" } | get id)
    let motion = (xrpc $owner createDocument { context_id: $group, parent_id: $motions, kind: "policy", title: "Kontingent", mutable: false } | get id)
    let poll = (xrpc $owner openPoll { parent_id: $motion, title: "Kontingent", options: ["for", "imod", "blank"], blank: true } | get id)
    xrpc $owner setProjector { context_id: $group, active_id: $poll } | ignore
    let list = (xrpc $owner createSpeakerList { context_id: $group, name: "Talerliste" } | get id)
    xrpc $member joinSpeakerList { list_id: $list } | ignore
    let canvas = (xrpc $owner createCanvas { parent_id: $group, name: "Tavle", width: 16, height: 16, cooldown: 0 } | get id)
    xrpc $member paintCell { canvas: $canvas, x: 3, y: 4, colour: 5 } | ignore
    { group: $group, folder: $folder, page: $page, motion: $motion, poll: $poll }
}

# A real pointer click at an offset from the middle of the element `css` finds.
def click-at [sid: string, css: string, x: int, y: int] {
    let found = (wd-post $"/session/($sid)/element" { using: "css selector", value: $css })
    let element = (try { $found | get value | values | first } catch { "" })
    if ($element | is-empty) { return }
    let origin = { "element-6066-11e4-a52e-4f735466cecf": $element }
    wd-post $"/session/($sid)/actions" { actions: [{
        type: "pointer", id: "mouse", parameters: { pointerType: "mouse" },
        actions: [
            { type: "pointerMove", duration: 0, origin: $origin, x: $x, y: $y }
            { type: "pointerDown", button: 0 }
            { type: "pointerUp", button: 0 }
        ]
    }] } | ignore
}

# Type into a text input as a person would: set it, and say so.
def type-into [sid: string, css: string, text: string] {
    js $sid $"const box = [...document.querySelectorAll\(($css | to json -r)\)].pop\(\); if \(!box\) return 0; Object.getOwnPropertyDescriptor\(HTMLInputElement.prototype, 'value'\).set.call\(box, ($text | to json -r)\); box.dispatchEvent\(new Event\('input', { bubbles: true }\)\); return 1;" | ignore
}

def main [
    --keep   # Leave the AppView, the dev server and the browser running
    --shots  # Save a PNG of each screen to ./screenshots/appview
] {
    let proj = ($env.FILE_PWD | path dirname)
    cd $proj
    let logs = (^mktemp -d | str trim)
    for cmd in [dx curl cargo fuser] {
        if (which $cmd | is-empty) { log-fail $"Required command not found: ($cmd)"; exit 2 }
    }
    for port in [$WD_PORT $SERVE_PORT $API_PORT] { kill-port $port }

    log-info "Building the dev AppView..."
    let built = (do -i { ^cargo build --quiet -p appview-dev --manifest-path crates/Cargo.toml } | complete)
    if $built.exit_code != 0 { log-fail "appview-dev did not build"; print -e $built.stderr; exit 2 }
    let api_pid = (start $"APPVIEW_FRONTEND_ORIGINS=(app) crates/target/debug/appview-dev --port ($API_PORT) did:plc:owner did:plc:member" $"($logs)/appview.json")
    sleep 2sec
    let sessions = (try { open $"($logs)/appview.json" | get sessions } catch { null })
    if $sessions == null { log-fail "the dev AppView did not start"; cleanup "" [$api_pid]; exit 2 }
    let owner = ($sessions | get "did:plc:owner")
    let member = ($sessions | get "did:plc:member")

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
    if not $built_ok {
        log-fail "dx serve did not finish its first build"
        print -e (try { open --raw $"($logs)/dx.log" | lines | last 20 | str join "\n" } catch { "" })
        cleanup "" $pids
        exit 2
    }

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

    let ids = (seed $owner $member)
    let shots_dir = $"($proj)/screenshots/appview"
    if $shots { mkdir $shots_dir }
    mut passed = 0
    mut failed = 0

    # Each check is a name, where to go, and text that has to turn up there.
    go $sid "/"
    let home_ok = (wait-for-text $sid "Log in" 120)
    if $home_ok { $passed = $passed + 1; log-ok "signed out: the home draws" } else { $failed = $failed + 1; log-fail "signed out: the home did not draw" }
    go $sid "/user/login"
    sleep 2sec
    let form = (js $sid 'return !!document.getElementById("auth-handle")')
    if $form == true { $passed = $passed + 1; log-ok "signing in asks for a handle" } else { $failed = $failed + 1; log-fail "no handle field on the sign-in screen" }

    # What `finish_sign_in` stores once a provider has sent the browser back.
    let stored = ({
        user: { id: "did:plc:owner", email: "@owner.test", display_name: "Owner", avatar_url: "" },
        access_token: $owner, refresh_token: $owner, node_id: null,
        access_token_expires_at: 4102444800000.0
    } | to json -r)
    js $sid $"localStorage.setItem\('wiki_session', ($stored | to json -r)\); return 1" | ignore

    let screens = [
        [name path text];
        ["the group lists its folders" "/hovedbestyrelsen" "Forslag"]
        ["a page shows its text" "/hovedbestyrelsen/bilag/dagsorden" "Valg af dirigent og referent"]
        ["and its thread, with who wrote it" "/hovedbestyrelsen/bilag/dagsorden" "Enig i dagsordenen"]
        ["the feed" "/hovedbestyrelsen?app=feed" "Enig i dagsordenen"]
        ["the open poll is on the vote screen" "/hovedbestyrelsen?app=vote" "Kontingent"]
        ["the speaker list has its speaker" "/hovedbestyrelsen?app=speak" "Member"]
        ["the canvas counts its painted cell" "/hovedbestyrelsen?app=canvas" "1 / 256"]
        ["the roster" "/hovedbestyrelsen?app=member" "Medlem"]
        ["the profile names the account" "/profile/did:plc:owner" "@owner.test"]
    ]
    for screen in $screens {
        go $sid $screen.path
        if (wait-for-text $sid $screen.text 30) {
            $passed = $passed + 1
            log-ok $screen.name
        } else {
            $failed = $failed + 1
            log-fail $"($screen.name): ($screen.text | to json -r) never turned up at ($screen.path)"
        }
        if $shots { shot $sid $"($shots_dir)/($screen.name | str replace -a ' ' '-').png" }
    }

    # A comment written in the page: shown once, and not left as "sending".
    go $sid "/hovedbestyrelsen/bilag/dagsorden"
    wait-for-text $sid "Enig i dagsordenen" 30 | ignore
    js $sid '
        const box = document.querySelector("textarea");
        const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set;
        set.call(box, "Skrevet i browseren");
        box.dispatchEvent(new Event("input", { bubbles: true }));
        return 1;' | ignore
    sleep 500ms
    js $sid 'const i = [...document.querySelectorAll("button .material-icons")].find(i => i.textContent.trim() === "send"); i.closest("button").click(); return 1;' | ignore
    sleep 5sec
    let rows = (js $sid 'return document.body.innerText.split("Skrevet i browseren").length - 1')
    let sending = (js $sid 'return document.body.innerText.includes("Sending")')
    if $rows == 1 and $sending == false {
        $passed = $passed + 1
        log-ok "a comment written in the page lands, once"
    } else {
        $failed = $failed + 1
        log-fail $"a comment written in the page: shown ($rows) times, still sending: ($sending)"
    }

    # Written elsewhere while this page is open: it arrives by itself.
    xrpc $member postComment { on_id: $ids.page, text: "Sendt fra en anden enhed" } | ignore
    if (wait-for-text $sid "Sendt fra en anden enhed" 15) {
        $passed = $passed + 1
        log-ok "a comment written elsewhere arrives without a reload"
    } else {
        $failed = $failed + 1
        log-fail "a comment written elsewhere never arrived"
    }
    go $sid "/hovedbestyrelsen/bilag"
    wait-for-text $sid "Dagsorden" 30 | ignore
    xrpc $owner createDocument { context_id: $ids.group, parent_id: $ids.folder, kind: "document", title: "Referat fra sidst", mutable: false } | ignore
    if (wait-for-text $sid "Referat fra sidst" 15) {
        $passed = $passed + 1
        log-ok "a page made elsewhere turns up in the open folder"
    } else {
        $failed = $failed + 1
        log-fail "a page made elsewhere never turned up in the open folder"
    }

    # A ballot cast through the screen.
    go $sid "/hovedbestyrelsen?app=vote"
    wait-for-text $sid "Kontingent" 30 | ignore
    js $sid 'const o = [...document.querySelectorAll(".ballot-option")].find(e => e.innerText.trim() === "For"); if (o) o.click(); return 1;' | ignore
    sleep 500ms
    js $sid 'const v = document.querySelector("button.btn-cast"); if (v) v.click(); return 1;' | ignore
    if (wait-for-text $sid "You have voted" 15) {
        $passed = $passed + 1
        log-ok "a ballot cast on the vote screen is counted"
    } else {
        $failed = $failed + 1
        log-fail "the vote screen never said the ballot was in"
    }

    # A secret ballot. The blinding and the RSA run in the browser's wasm, and the
    # ballot goes in with no session, so the server's count is the proof.
    xrpc $owner closePoll { id: $ids.poll } | ignore
    let secret = (xrpc $owner openPoll { parent_id: $ids.motion, title: "Hemmelig afstemning", options: ["for", "imod", "blank"], blank: true, secret: true } | get id)
    xrpc $owner setProjector { context_id: $ids.group, active_id: $secret } | ignore
    go $sid "/hovedbestyrelsen?app=vote"
    wait-for-text $sid "Hemmelig afstemning" 30 | ignore
    js $sid 'const o = [...document.querySelectorAll(".ballot-option")].find(e => e.innerText.trim() === "For"); if (o) o.click(); return 1;' | ignore
    sleep 500ms
    js $sid 'const v = document.querySelector("button.btn-cast"); if (v) v.click(); return 1;' | ignore
    let said_so = (wait-for-text $sid "You have voted" 30)
    # The screen says so at once, before the ballot is in, so the count is what
    # is waited for: leaving the page sooner would take the request with it.
    mut counted = { ballots: 0, counts: [] }
    mut waited = 0
    while $waited < 60 and $counted.ballots != 1 {
        $counted = (^curl -s -H $"authorization: Bearer ($owner)" $"(api).getPoll?id=($secret)" | from json | select ballots counts)
        sleep 500ms
        $waited = $waited + 1
    }
    let counted = $counted
    if $said_so and ($counted.ballots == 1) and ($counted.counts == [1 0 0]) {
        $passed = $passed + 1
        log-ok "a secret ballot is blinded, signed and cast from the browser, and counted"
    } else {
        $failed = $failed + 1
        log-fail $"a secret ballot: the screen said so: ($said_so), the server counts ($counted.ballots) ballots as ($counted.counts)"
    }

    # Joining the queue: shown at once, and the row shown early goes away.
    go $sid "/hovedbestyrelsen?app=speak"
    wait-for-text $sid "Talerliste" 30 | ignore
    js $sid 'const f = document.querySelector(".speak-join-fab"); if (f) f.click(); return 1;' | ignore
    sleep 1sec
    js $sid 'const i = [...document.querySelectorAll(".speak-join-item")].pop(); if (i) i.click(); return 1;' | ignore
    let queued = (wait-for-text $sid "Owner" 15)
    sleep 4sec
    let early = (js $sid 'return document.querySelectorAll(".list-item.is-pending").length')
    if $queued and $early == 0 {
        $passed = $passed + 1
        log-ok "joining the speaker list lands, once"
    } else {
        $failed = $failed + 1
        log-fail $"joining the speaker list: in the queue: ($queued), rows still pending: ($early)"
    }

    # A cell painted with a real click.
    go $sid "/hovedbestyrelsen?app=canvas"
    wait-for-text $sid "1 / 256" 30 | ignore
    js $sid 'const s = document.querySelectorAll(".pixel-swatch"); if (s[2]) s[2].click(); return 1;' | ignore
    sleep 500ms
    click-at $sid "canvas.pixel-board" 100 60
    if (wait-for-text $sid "2 / 256" 15) {
        $passed = $passed + 1
        log-ok "a tap on the canvas paints a cell"
    } else {
        $failed = $failed + 1
        log-fail "the canvas never counted a second cell"
    }

    # A page made from the folder, written in the editor, saved, and read back.
    go $sid "/hovedbestyrelsen/bilag"
    wait-for-text $sid "Dagsorden" 30 | ignore
    js $sid 'const b = document.querySelector("button.add-action"); if (b) b.click(); return 1;' | ignore
    sleep 1sec
    type-into $sid "[role=dialog] input[type=text], dialog input[type=text], .dialog input[type=text]" "Beretning 2026"
    sleep 500ms
    js $sid 'const add = [...document.querySelectorAll("button.btn-primary")].find(b => b.innerText.trim() === "Add"); if (add) add.click(); return 1;' | ignore
    sleep 6sec
    let landed = (js $sid 'return location.pathname + location.search')
    js $sid 'const ed = document.querySelector("[contenteditable=true]"); if (!ed) return 0; ed.focus(); document.execCommand("insertText", false, "Aaret gik godt."); return 1;' | ignore
    sleep 1sec
    js $sid 'const save = [...document.querySelectorAll("button")].find(b => b.innerText.replace(/\s+/g, " ").trim() === "save Save"); if (save) save.click(); return 1;' | ignore
    sleep 4sec
    go $sid "/hovedbestyrelsen/bilag/beretning_2026"
    let read_back = (wait-for-text $sid "Aaret gik godt." 30)
    # An owner's save sends the page's day back; it must not reset its time.
    let aged = (js $sid 'return /\d+ hours? ago/.test(document.body.innerText)')
    if ($landed | str contains "beretning_2026") and $read_back and $aged == false {
        $passed = $passed + 1
        log-ok "a page is made, written, saved and read back, dated when it was made"
    } else {
        $failed = $failed + 1
        log-fail $"a new page: landed at ($landed), read back: ($read_back), dated hours ago: ($aged)"
    }

    # The app's own errors, as the browser console heard them.
    let errors = (try { open --raw $"($logs)/gecko.log" | lines | where { |l| $l =~ '%cERROR%c' } } catch { [] })
    if ($errors | is-empty) {
        $passed = $passed + 1
        log-ok "the app logged no error"
    } else {
        $failed = $failed + 1
        log-fail $"the app logged ($errors | length) errors, the first: ($errors | first | str substring 0..300)"
    }

    log-info $"($passed) passed, ($failed) failed. Logs: ($logs)"
    if $keep {
        log-info $"Left running: the app on (app), the AppView on :($API_PORT), WebDriver on :($WD_PORT), session ($sid)"
    } else {
        cleanup $sid $pids
    }
    if $failed > 0 { exit 1 }
}
