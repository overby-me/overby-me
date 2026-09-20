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
# behind its back. Of the ballot board (`crates/appview/src/board.rs`,
# `crates/board-mirror`): a closed group's board in its space, kept and
# recounted by a member. When the alpha moves, this is what says where.
#
# Usage: nu scripts/test-spaces.nu [--keep]
# Exit codes: 0 passed · 1 failed · 2 setup failed (docker, the image, the PDS)

use spaces-pds.nu *

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset)  ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset)  ($msg | str join ' ')" }

def main [
    --keep  # Leave the PDS and the directory running after
] {
    let proj = ($env.FILE_PWD | path dirname)
    cd $proj
    log-info "Starting the directory stand-in and the alpha's PDS..."
    let version = (try { start-spaces-pds ($proj | path join "crates") } catch { |e| log-fail $e.msg; exit 2 })
    log-info $"PDS ($version) is up. Running the crate against it..."

    $env.SPACES_ALPHA_PDS = $"http://localhost:($PDS_PORT)"
    $env.SPACES_ALPHA_PLC = $"http://127.0.0.1:($PLC_PORT)"
    let suites = [
        [-p atproto-spaces --test alpha]
        [-p appview --lib spaces::tests::the_wiki_in_a_real_pds]
        [-p appview --lib board::tests::a_member_mirrors_a_board_out_of_a_real_space]
    ]
    mut failed = false
    for suite in $suites {
        let ran = (do -i { ^cargo test --manifest-path crates/Cargo.toml ...$suite -- --ignored --nocapture } | complete)
        print $ran.stdout
        if $ran.exit_code != 0 { print -e $ran.stderr; $failed = true }
    }
    if not $keep { stop-spaces-pds }
    if $failed { log-fail $"against PDS ($version)"; exit 1 }
    log-info $"Passed against PDS ($version)."
}
