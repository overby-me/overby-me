#!/usr/bin/env nu

# Build the frontend and drop it on a statichost.eu site.
#
# A statichost drop REPLACES the whole site, so this is not "upload the new
# files": whatever is not in the zip is gone. That is what makes `symbols/`
# fragile. The backend fetches those sidecars to turn a crash report's raw wasm
# offsets into source lines, and `dx build` sometimes clears its output
# directory first, so a build can arrive with only its own sidecar and every
# earlier report is stranded. They are copied aside before the build and put
# back after.
#
# Credentials come from the environment and are never written here:
#
#   BETTERSTACK_SOURCE_TOKEN  presence enables `--features remote-logging` in
#                             the build (its VALUE is not baked into the wasm;
#                             the real ingest token lives on the backend)
#   STATICHOST_APIKEY         the drop credential
#
# Usage:
#   BETTERSTACK_SOURCE_TOKEN=... STATICHOST_APIKEY=... \
#     nu scripts/deploy-frontend.nu wiki-prod

# Fail with a message rather than a stack trace.
def fail [msg: string] {
    error make --unspanned { msg: $"deploy-frontend: ($msg)" }
}

def main [
    site: string = "wiki-prod" # statichost.eu site: wiki-prod = radikal.wiki, radikal-wiki = dev
] {
    let root = ($env.FILE_PWD | path dirname)
    let public = ($root | path join "target/dx/wiki-dioxus/release/web/public")

    # A bundle without remote logging is a bundle whose crashes nobody sees, and
    # the difference is invisible once it is deployed: the site looks identical
    # and simply stops reporting. Refuse rather than ship one by accident.
    if ($env.BETTERSTACK_SOURCE_TOKEN? | is-empty) {
        fail "BETTERSTACK_SOURCE_TOKEN is not set, so the build would ship without remote logging and report nothing. Export it and run again."
    }
    if ($env.STATICHOST_APIKEY? | is-empty) {
        fail "STATICHOST_APIKEY is not set. It is the statichost.eu drop credential and is deliberately not in the repo."
    }

    # `just build` stamps version.json and sw.js from `git rev-parse HEAD` and
    # appends `-dirty` over an unclean tree. A deployed `-dirty` build names a
    # commit that does not describe what is running, which is worse for reading
    # a crash report than naming none.
    let dirty = (^git status --porcelain --untracked-files=no | complete | get stdout | str trim)
    if ($dirty | is-not-empty) {
        fail $"the working tree has uncommitted changes, so the build would be stamped -dirty. Commit first.\n($dirty)"
    }

    # Keep every sidecar that is currently built, to survive a `dx build` that
    # clears the directory.
    let symbols = ($public | path join "symbols")
    let keep = (mktemp -d)
    if ($symbols | path exists) {
        ls $symbols | where name =~ '\.debug\.wasm$' | each {|f| cp $f.name $keep } | ignore
    }

    print $"building ($site) ..."
    ^just build
    if $env.LAST_EXIT_CODE != 0 { fail "the build failed" }

    # Put back anything the build dropped. `cp -n` semantics: never overwrite the
    # sidecar this build just produced.
    mkdir $symbols
    ls $keep | each {|f|
        let dest = ($symbols | path join ($f.name | path basename))
        if not ($dest | path exists) { cp $f.name $dest }
    } | ignore
    rm -rf $keep

    # The env var above only proves the token was set for THIS process. What
    # matters is the artifact, so ask the wasm. `sw_build` is a key of the report
    # payload in logging.rs, which is compiled only under `remote-logging`.
    let index = ($public | path join "index.html")
    if not ($index | path exists) { fail $"no bundle at ($public)" }
    let wasm_name = (
        open --raw $index
        | parse --regex 'wiki-dioxus_bg-(?<h>[a-z0-9]+)\.wasm'
        | get h.0?
    )
    if ($wasm_name | is-empty) { fail "could not find the wasm referenced by index.html" }
    let wasm = ($public | path join $"assets/wiki-dioxus_bg-($wasm_name).wasm")
    let logged = (^grep -qa "sw_build" $wasm | complete | get exit_code)
    if $logged != 0 {
        fail "the built wasm carries no remote-logging payload, so this bundle would report nothing. Check that BETTERSTACK_SOURCE_TOKEN reached the build."
    }

    let stamp = (open ($public | path join "version.json") | get commit)
    let zip = (mktemp --tmpdir --suffix .zip)
    cd $public
    ^zip -qr $zip .
    if $env.LAST_EXIT_CODE != 0 { fail "zip failed" }

    print $"dropping ($stamp) on ($site) ..."
    let out = (
        ^curl --fail-with-body -sS -X POST $"https://builder.statichost.eu/($site)/drop"
            -H $"Authorization: Bearer ($env.STATICHOST_APIKEY)"
            -H "Content-Type: application/zip"
            --data-binary $"@($zip)"
        | complete
    )
    rm -f $zip
    if $out.exit_code != 0 or not ($out.stdout | str contains "Build succeeded") {
        fail $"the drop failed:\n($out.stdout)($out.stderr)"
    }

    # The site is only deployed once it serves the commit we built. Anything
    # else means the drop landed somewhere other than where it is being read.
    let host = if $site == "wiki-prod" { "https://radikal.wiki" } else { "https://dev.radikal.wiki" }
    let live = (^curl -s $"($host)/version.json" | complete | get stdout)
    print $"live: ($live | str trim)"
    if not ($live | str contains $stamp) {
        fail $"($host) does not serve ($stamp) yet. It may still be publishing; check again before trusting it."
    }
    print $"deployed ($stamp) to ($host)"
}
