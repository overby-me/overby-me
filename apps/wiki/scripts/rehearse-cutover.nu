#!/usr/bin/env nu
# The cutover (docs/cutover-runbook.md), rehearsed on this machine from a dump
# that is already taken:
#
#   <dir>/snapshot.json    from scripts/dump-interim-snapshot.nu
#   <dir>/files/           from scripts/dump-interim-files.nu (optional)
#
#   extract -> appview import -> import-files -> verify (the gates)
#     -> every carried account returns -> a smoke test over HTTP
#
# Everything it writes stays in <dir>, which holds people's addresses: keeping
# it private and removing it afterwards is the operator's. It prints counts,
# statuses and ids, and never a name, an address or a page's text.
#
# Usage: nu scripts/rehearse-cutover.nu <dir> [--port 8190]

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset)  ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset)  ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset)  ($msg | str join ' ')" }

# One call, answered with its status whatever that is.
def call [url: string, token?: string] {
    let headers = (if ($token | is-empty) { {} } else { {authorization: $"Bearer ($token)"} })
    http get --full --allow-errors --headers $headers $url
}

def post [url: string, token: string, body: record] {
    http post --full --allow-errors --content-type application/json --headers {authorization: $"Bearer ($token)"} $url ($body | to json)
}

def refused [status: int]: nothing -> bool { $status in [401 403 404] }

def main [dir: path, --port: int = 8190] {
    let dir = ($dir | path expand)
    let snapshot = ($dir | path join "snapshot.json")
    if not ($snapshot | path exists) {
        log-fail $"no ($snapshot): take the dump first \(scripts/dump-interim-snapshot.nu\)"
        exit 2
    }
    let crates = ($env.FILE_PWD | path dirname | path join "crates")
    let bin = ($crates | path join "target" "release")

    log-info "Building the extractor, the AppView and the dev AppView..."
    cd $crates
    ^cargo build --quiet --release -p migration-extractor -p appview -p appview-dev
    cd $dir

    log-info "Extracting..."
    ^($bin | path join "extract") snapshot.json

    let stage = ($dir | path join "stage")
    rm -rf $stage
    mkdir $stage
    $env.APPVIEW_DB = ($stage | path join "wiki.db")
    $env.APPVIEW_BLOB_DIR = ($stage | path join "blobs")
    # Nothing is signed with it that outlives the rehearsal.
    $env.APPVIEW_SECRET = (random chars --length 48)
    let appview = ($bin | path join "appview")
    let files = ($dir | path join "files")
    let has_files = ($files | path join "manifest.json" | path exists)

    log-info "Loading..."
    ^$appview import extraction.json
    if $has_files {
        ^$appview import-files extraction.json files | lines | where { |l| $l !~ '^nothing points at' } | each { |l| print $l } | ignore
    }

    log-info "The gates..."
    let gates = (if $has_files { ^$appview verify extraction.json files | complete } else { ^$appview verify extraction.json | complete })
    print $gates.stdout
    mut failed = (if $gates.exit_code == 0 { 0 } else { 1 })

    # On a copy: the loaded datastore stays as the load left it.
    let returns = ($dir | path join "returns")
    rm -rf $returns
    mkdir $returns
    ls $stage | where type == file | each { |f| cp $f.name $returns } | ignore
    let hello = ($returns | path join "hello.json")
    let people = (0..39 | each { |i| $"did:plc:returned($i)" })
    log-info "Every carried account returns, and the datastore is served..."
    let server = (job spawn {
        ^($bin | path join "appview-dev") --port $port --db ($returns | path join "wiki.db") --everyone-returns ...$people did:plc:stranger out> $hello err> ($returns | path join "serve.log")
    })
    mut waited = 0
    while $waited < 600 and ((not ($hello | path exists)) or ((ls $hello | get 0.size) == 0B)) {
        sleep 1sec
        $waited = $waited + 1
    }
    if not ($hello | path exists) or ((ls $hello | get 0.size) == 0B) {
        log-fail "the dev AppView did not come up; see returns/serve.log"
        job kill $server
        exit 1
    }
    let said = (open $hello)
    let back = $said.everyone_returns
    if ($back.failed | is-empty) and $back.seats_still_held_by_a_carried_account == 0 {
        log-ok $"($back.returned) accounts returned and found ($back.seats) seats; none is left with an account nobody can sign in to"
    } else {
        $failed = $failed + 1
        log-fail $"($back.failed | length) accounts could not return, ($back.seats_still_held_by_a_carried_account) seats are still held by one that did: ($back.failed | to json -r)"
    }

    let api = $"($said.url)/xrpc/com.example.wiki"
    let stranger = ($said.sessions | get "did:plc:stranger")
    let ex = (open extraction.json)
    mut checks = []

    let open_to_all = ($ex.contexts | where visibility == "public")
    for c in $open_to_all {
        let r = (call $"($api).getNode?path=($c.path | url encode)")
        $checks = ($checks | append {what: "a public context opens to nobody", ok: ($r.status == 200), saw: $"status ($r.status)"})
    }
    let closed = ($ex.contexts | where visibility != "public" and path != "" | first)
    let r = (call $"($api).getNode?path=($closed.path | url encode)")
    $checks = ($checks | append {what: "a closed context does not open to nobody", ok: (refused $r.status), saw: $"status ($r.status)"})

    # The returned account with the most seats, among those given a session.
    let seated = ($people | each { |did|
        let token = ($said.sessions | get $did)
        let mine = (call $"($api).listContexts?scope=mine" $token)
        {did: $did, token: $token, contexts: ($mine.body.contexts? | default [] | get id)}
    } | sort-by { |p| $p.contexts | length } | last)
    let member = $seated.token
    let mine = $seated.contexts
    $checks = ($checks | append {what: "a returned member has the seats of their old account", ok: (($mine | length) > 0), saw: $"($mine | length) contexts"})

    if ($mine | is-not-empty) {
        let r = (call $"($api).listMembers?context=($mine | first)" $member)
        $checks = ($checks | append {what: "the member list of a context of theirs answers", ok: ($r.status == 200 and ($r.body.total? | default 0) > 0), saw: $"status ($r.status), ($r.body.total? | default 0) members"})
        let r = (call $"($api).listMembers?context=($mine | first)" $stranger)
        $checks = ($checks | append {what: "and not to a stranger", ok: (refused $r.status), saw: $"status ($r.status)"})
    }

    let theirs = ($ex.documents | where { |d| $d.context_id in $mine and ($d.deleted_at? | is-empty) })
    let several = ($theirs | where { |d| ($d.authors | length) >= 2 })
    if ($several | is-not-empty) {
        let d = ($several | first)
        let r = (call $"($api).getNode?id=($d.id)" $member)
        let authors = ($r.body.node?.authors? | default [] | length)
        $checks = ($checks | append {what: "a document with several authors opens with all of them", ok: ($r.status == 200 and $authors == ($d.authors | length)), saw: $"status ($r.status), ($authors) of ($d.authors | length) authors"})
        let r = (call $"($api).getNode?id=($d.id)" $stranger)
        $checks = ($checks | append {what: "and does not open to a stranger", ok: (refused $r.status), saw: $"status ($r.status)"})
        let r = (call $"($api).getNode?path=($d.path | url encode)" $member)
        $checks = ($checks | append {what: "and is where its old URL says", ok: ($r.status == 200 and ($r.body.node?.id? | default "") == $d.id), saw: $"status ($r.status)"})
        let r = (post $"($api).postComment" $member {on_id: $d.id, text: "rehearsal"})
        $checks = ($checks | append {what: "a comment lands on it", ok: ($r.status == 200), saw: $"status ($r.status)"})
        let r = (call $"($api).getComments?on=($d.id)" $member)
        let thread = ($r.body.comments? | default [] | length)
        $checks = ($checks | append {what: "and is read back", ok: ($r.status == 200 and $thread >= 1), saw: $"($thread) comments"})
        let words = ($d.title | split row --regex '[^\p{L}]+' | where { |w| ($w | str length) > 3 } | sort-by { |w| $w | str length })
        if ($words | is-not-empty) {
            let word = ($words | last | url encode)
            let r = (call $"($api).search?q=($word)" $member)
            let hits = ($r.body.hits? | default [])
            $checks = ($checks | append {what: "search finds it by a word of its title", ok: ($d.id in ($hits | get id)), saw: $"status ($r.status), ($hits | length) hits"})
            let r = (call $"($api).search?q=($word)" $stranger)
            let hits = ($r.body.hits? | default [])
            $checks = ($checks | append {what: "and a stranger finds nothing of it", ok: ($d.id not-in ($hits | get id)), saw: $"($hits | length) hits"})
        }
    }

    let with_a_file = ($theirs | where { |d| ($d.data?.fileId? | default "") != "" })
    if $has_files and ($with_a_file | is-not-empty) {
        let id = ($with_a_file | first | get data.fileId)
        let listed = (open ($files | path join "manifest.json") | where id == $id | get 0?.size? | default (-1))
        let got = ($returns | path join "fetched")
        let status = (^curl -s -o $got -w "%{http_code}" -H $"authorization: Bearer ($member)" $"($said.url)/blob/($id)" | str trim)
        let size = (ls $got | get 0.size | into int)
        $checks = ($checks | append {what: "a file is served to a member, whole", ok: ($status == "200" and $size == $listed), saw: $"status ($status), ($size) of ($listed) bytes"})
        rm -f $got
        let status = (^curl -s -o /dev/null -w "%{http_code}" -H $"authorization: Bearer ($stranger)" $"($said.url)/blob/($id)" | str trim | into int)
        $checks = ($checks | append {what: "and refused to a stranger", ok: (refused $status), saw: $"status ($status)"})
        let status = (^curl -s -o /dev/null -w "%{http_code}" $"($said.url)/blob/($id)" | str trim | into int)
        $checks = ($checks | append {what: "and to nobody", ok: (refused $status), saw: $"status ($status)"})
    }
    job kill $server

    for c in $checks {
        if $c.ok { log-ok $"($c.what): ($c.saw)" } else { log-fail $"($c.what): ($c.saw)" }
    }
    $failed = $failed + ($checks | where not ok | length)
    if $failed == 0 {
        log-info "The rehearsal is green. What was left behind on purpose is in report.json, and is for a person to read."
    } else {
        log-info $"($failed) things are red: no flip."
        exit 1
    }
}
