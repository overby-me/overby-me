#!/usr/bin/env nu
# The move into atproto spaces (docs/atproto-spaces-redesign.md, S6), rehearsed
# on this machine from a cutover already rehearsed:
#
#   <dir>/stage/wiki.db, <dir>/stage/blobs/    from scripts/rehearse-cutover.nu
#
#   the alpha's PDS in a container -> a made-up organization on it
#     -> appview mirror-spaces --bytes: every page, comment, reaction and file
#        as a record; every space read back; every row rebuilt from its record
#        alone; every file's bytes fetched back
#
# It answers what only real data can: which pages are past what a PDS takes as
# one record, which files are past its blob limit, and whether the wiki
# rebuilds from its records. The datastore is written to (what was mirrored is
# remembered in it), so it works on a copy.
#
# The container holds a copy of the wiki for as long as it runs, in memory, and
# is removed at the end. It prints counts, ids and field names, and never a
# name, an address or a page's text.
#
# Usage: nu scripts/rehearse-spaces.nu <dir> [--blob-limit 26214400]
# Exit codes: 0 green · 1 not green · 2 setup failed

use spaces-pds.nu *

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset)  ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset)  ($msg | str join ' ')" }

def main [
    dir: path
    --blob-limit: int = 26214400  # The largest file the rehearsed PDS takes, in bytes (25 MiB)
] {
    let dir = ($dir | path expand)
    let staged = ($dir | path join "stage" "wiki.db")
    if not ($staged | path exists) {
        log-fail $"no ($staged): rehearse the cutover first \(scripts/rehearse-cutover.nu\)"
        exit 2
    }
    let crates = ($env.FILE_PWD | path dirname | path join "crates")
    log-info "Building the AppView..."
    let built = (do -i { ^cargo build --quiet --release -p appview --manifest-path ($crates | path join "Cargo.toml") } | complete)
    if $built.exit_code != 0 { log-fail "the AppView did not build"; print -e $built.stderr; exit 2 }

    # On a copy: the rehearsed cutover stays what it was.
    let work = ($dir | path join "spaces")
    rm -rf $work
    mkdir $work
    for file in (glob ($dir | path join "stage" "wiki.db*")) { cp $file $work }

    log-info "Starting the directory stand-in and the alpha's PDS..."
    let version = (try { start-spaces-pds $crates $blob_limit } catch { |e| log-fail $e.msg; exit 2 })
    let pds = $"http://localhost:($PDS_PORT)"
    let password = (random chars --length 32)
    let handle = $"wiki(random chars --length 8 | str lowercase).test"
    let made = (http post --full --allow-errors --content-type application/json $"($pds)/xrpc/com.atproto.server.createAccount" (
        {handle: $handle, email: $"($handle)@rehearsal.invalid", password: $password} | to json))
    if $made.status != 200 { log-fail $"the PDS made no organization: ($made.status)"; stop-spaces-pds; exit 2 }

    log-info $"PDS ($version). Mirroring, reading back, rebuilding..."
    $env.APPVIEW_DB = ($work | path join "wiki.db")
    $env.APPVIEW_BLOB_DIR = ($dir | path join "stage" "blobs")
    $env.APPVIEW_SECRET = (random chars --length 48)
    $env.APPVIEW_PLC_URL = $"http://127.0.0.1:($PLC_PORT)"
    $env.APPVIEW_SPACES_PDS = $pds
    $env.APPVIEW_SPACES_IDENTIFIER = $handle
    $env.APPVIEW_SPACES_PASSWORD = $password
    # Never asked: the organization reads its own spaces without it.
    $env.APPVIEW_SPACES_SERVICE = "did:web:rehearsal.invalid#wiki_appview"
    $env.APPVIEW_SPACES_BLOB_LIMIT = ($blob_limit | into string)
    $env.RUST_LOG = "warn"
    let ran = (do -i { ^($crates | path join "target" "release" "appview") mirror-spaces --bytes } | complete)
    stop-spaces-pds

    # What the PDS would not take, by id and size, is in the log lines.
    let refused = ($ran.stdout | lines | where { |l| $l starts-with "{" } | each { |l| try { $l | from json } catch { null } } | compact
        | where { |l| ($l.fields?.message? | default "") =~ "refused|past what the PDS takes" } | each { |l| $l.fields.message })
    for line in $refused { print $"  ($line)" }
    for line in ($ran.stdout | lines | where { |l| not ($l starts-with "{") }) { print $line }
    if $ran.exit_code != 0 {
        log-fail "the spaces are not the index yet: see above"
        exit 1
    }
    log-info "Green: the wiki is in its spaces, and rebuilds from them."
}
