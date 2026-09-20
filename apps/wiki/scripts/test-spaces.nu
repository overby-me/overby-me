#!/usr/bin/env nu

# test-spaces.nu: the wiki's use of atproto spaces against a real spaces PDS.
#
# atproto spaces are an alpha that promises breaking changes weekly. This pulls
# the alpha's PDS image, runs it in a container beside the stand-in directory
# (`crates/fake-plc`), and runs the ignored tests against the two. Of
# `crates/atproto-spaces`: a space under an organization's account with a
# managing app deciding access, records, the credential flow, sync held to
# signed commits, write notifications, and a file. Of the AppView
# (`crates/appview/src/spaces.rs`): a wiki mirrored into its spaces and read
# back, the PDS asking the AppView itself who gets in, and a space deleted
# behind its back. When the alpha moves, this is what says where.
#
# Usage: nu scripts/test-spaces.nu [--keep]
# Exit codes: 0 passed · 1 failed · 2 setup failed (docker, the image, the PDS)

const PLC_PORT = 2582
const PDS_PORT = 2583
const IMAGE = "ghcr.io/bluesky-social/atproto:pds-spaces-alpha"
const NAME = "wiki-spaces-pds"

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset)  ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset)  ($msg | str join ' ')" }

def kill-port [port: int] {
    try { ^fuser -k $"($port)/tcp" | complete | ignore } catch { }
}

def cleanup [] {
    do -i { ^docker rm -f $NAME } | complete | ignore
    kill-port $PLC_PORT
}

def main [
    --keep  # Leave the PDS and the directory running after
] {
    let proj = ($env.FILE_PWD | path dirname)
    cd $proj
    for cmd in [docker cargo curl fuser] {
        if (which $cmd | where type == "external" | is-empty) { log-fail $"Required command not found: ($cmd)"; exit 2 }
    }
    cleanup

    log-info "Building the directory stand-in..."
    let built = (do -i { ^cargo build --quiet -p fake-plc --manifest-path crates/Cargo.toml } | complete)
    if $built.exit_code != 0 { log-fail "fake-plc did not build"; print -e $built.stderr; exit 2 }
    ^bash -c $'crates/target/debug/fake-plc --port ($PLC_PORT) > /dev/null 2>&1 < /dev/null &'

    log-info $"Starting ($IMAGE)..."
    # On the host's network, so that it reaches the directory and the test's own
    # listener, both on loopback. Nothing it writes outlives the container.
    let secret = (random chars --length 32)
    let rotation = (random binary 32 | encode hex | str lowercase)
    let settings = [
        "PDS_HOSTNAME=localhost" $"PDS_PORT=($PDS_PORT)" "PDS_DEV_MODE=true" "NODE_ENV=development"
        "PDS_DATA_DIRECTORY=/pds" "PDS_BLOBSTORE_DISK_LOCATION=/pds/blocks"
        $"PDS_DID_PLC_URL=http://127.0.0.1:($PLC_PORT)"
        $"PDS_JWT_SECRET=($secret)" $"PDS_ADMIN_PASSWORD=(random chars --length 24)"
        $"PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX=($rotation)"
        "PDS_SERVICE_HANDLE_DOMAINS=.test" "PDS_INVITE_REQUIRED=false"
    ]
    let flags = ($settings | each { |s| ["-e" $s] } | flatten)
    let started = (do -i { ^docker run -d --name $NAME --network host --tmpfs /pds ...$flags $IMAGE } | complete)
    if $started.exit_code != 0 { log-fail "docker could not start the PDS"; print -e $started.stderr; cleanup; exit 2 }

    mut up = false
    mut waited = 0
    while $waited < 60 and not $up {
        $up = ((do -i { ^curl -s -m 2 -o /dev/null -w "%{http_code}" $"http://localhost:($PDS_PORT)/xrpc/_health" } | complete).stdout | str trim) == "200"
        if not $up { sleep 1sec }
        $waited = $waited + 1
    }
    if not $up { log-fail "the PDS did not come up"; print -e (do -i { ^docker logs --tail 20 $NAME } | complete).stderr; cleanup; exit 2 }
    let version = (try { http get $"http://localhost:($PDS_PORT)/xrpc/_health" | get version } catch { "unknown" })
    log-info $"PDS ($version) is up. Running the crate against it..."

    $env.SPACES_ALPHA_PDS = $"http://localhost:($PDS_PORT)"
    $env.SPACES_ALPHA_PLC = $"http://127.0.0.1:($PLC_PORT)"
    let suites = [
        [-p atproto-spaces --test alpha]
        [-p appview --lib spaces::tests::the_wiki_in_a_real_pds]
    ]
    mut failed = false
    for suite in $suites {
        let ran = (do -i { ^cargo test --manifest-path crates/Cargo.toml ...$suite -- --ignored --nocapture } | complete)
        print $ran.stdout
        if $ran.exit_code != 0 { print -e $ran.stderr; $failed = true }
    }
    if not $keep { cleanup }
    if $failed { log-fail $"against PDS ($version)"; exit 1 }
    log-info $"Passed against PDS ($version)."
}
