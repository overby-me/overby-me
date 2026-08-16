#!/usr/bin/env nu
# Run the browser harness against an already-running `dx serve` (skips the build).
# Waits for any in-progress hot-reload rebuild (after a Rust edit) to settle first,
# then runs `test-browser.nu --firefox --reuse`. Extra flags are forwarded, so:
#
#   nu scripts/test-reuse.nu               # fast reuse run
#   nu scripts/test-reuse.nu --shots       # + capture screenshots
#
# Reads WIKI_EMAIL / WIKI_PASSWORD from the environment (never committed) to enable
# the authenticated tests. Start the server once with scripts/serve-up.nu.

const PORT = 8134
const LOG = "/tmp/wiki-dxserve.log"

def serving [] { (do -i { ^curl -sf -o /dev/null $"http://127.0.0.1:($PORT)/" } | complete).exit_code == 0 }

def main [...args: string] {
    cd ($env.FILE_PWD | path dirname)
    if not (serving) {
        print $"no dx serve on :($PORT) — run `nu scripts/serve-up.nu` first"
        exit 3
    }
    # Wait for any rebuild triggered by a recent edit to finish: the serve log's
    # modified time stops advancing for 2s (dx serve is idle).
    if ($LOG | path exists) {
        for _ in 1..120 {
            let a = (ls $LOG | get 0.modified)
            sleep 2sec
            let b = (ls $LOG | get 0.modified)
            if $a == $b { break }
        }
    }
    ^nu test-browser.nu --firefox --reuse ...$args
}
