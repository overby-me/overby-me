#!/usr/bin/env nu
# Start a persistent `dx serve` (once) so test runs can `--reuse` it and skip the
# ~minute rebuild. `dx serve` hot-reloads on edits; scripts/test-reuse.nu waits
# for that before running. Idempotent — does nothing if the server is already up.
#
#   nu scripts/serve-up.nu          # start (or confirm) the dev server on :8134

const PORT = 8134
const LOG = "/tmp/wiki-dxserve.log"

def serving [] { (do -i { ^curl -sf -o /dev/null $"http://127.0.0.1:($PORT)/" } | complete).exit_code == 0 }

def main [] {
    # Run from the project root (this script lives in ./scripts).
    cd ($env.FILE_PWD | path dirname)
    if (serving) {
        print $"dx serve already up on :($PORT)"
        return
    }
    # Detach so it outlives this script (nushell backgrounds through bash nohup).
    let pid = (^bash -c $'nohup dx serve --port ($PORT) > "($LOG)" 2>&1 & echo $!' | str trim)
    print $"started dx serve pid ($pid); waiting for the first build..."
    mut ready = false
    for _ in 1..240 {
        let built = (try { (open --raw $LOG) | str contains "Build completed" } catch { false })
        if $built and (serving) { $ready = true; break }
        sleep 1sec
    }
    if $ready {
        print $"dx serve ready on :($PORT)"
    } else {
        print "dx serve did not finish its first build:"
        if ($LOG | path exists) { print (open --raw $LOG | lines | last 15 | str join "\n") }
        exit 2
    }
}
