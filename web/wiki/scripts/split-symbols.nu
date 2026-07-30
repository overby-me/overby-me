#!/usr/bin/env nu
# Split the debug info out of the built wasm.
#
# The bundle is built WITH DWARF line tables so a crash can be traced back to a
# source line. Shipping those sections would add ~20 MB to every visit, so this
# writes two files instead:
#
#   public/assets/wiki-dioxus_bg-<hash>.wasm   stripped — the one that ships
#   public/symbols/<hash>.debug.wasm           the same module, DWARF intact
#
# Stripping only removes trailing custom sections, so the code section keeps its
# offset (verified: identical before and after). That is what makes the split
# safe — a stack frame's byte offset means the same thing in both files, so the
# symbols resolve against exactly the binary the reader was running.
#
# The pair is keyed by dx's own content hash, already in the filename and already
# in the wasm URL the browser reports, so no build-id section is needed. The
# backend fetches /symbols/<hash>.debug.wasm when a report arrives; see
# backend/src/symbolicate.rs.

const PUBLIC = "target/dx/wiki-dioxus/release/web/public"

# How many builds' worth of symbols to keep.
#
# dx never removes a superseded asset, so without this the directory gains ~26 MB
# per build forever and every deploy uploads the lot.
#
# Six, not three. Three assumed a reader reloads within a deploy or two, and a
# day of thirteen deploys disproved it: a crash arrived from a tab whose build
# was four deploys old, its symbols already pruned, and came back as raw offsets.
# What bounds this is how long a tab stays open, not how often we deploy. Six
# costs about 160 MB on the site and roughly a minute of upload.
#
# Losing an older one is not a failure: the backend fetches the sidecar, gets the
# site's SPA fallback (HTML, not a wasm module) instead, recognises it by the
# missing magic bytes and leaves the raw offsets alone. Unresolved beats wrong,
# which is why an older build is never resolved against a newer one's symbols —
# the offsets would land in whatever function now occupies that address.
const KEEP = 6

# Drop all but the newest KEEP sidecars, never the one this build just produced.
#
# What it removes is printed. A prune that stayed quiet would read as "everything
# is still there", which is the one thing it is not.
def prune_symbols [symbols_dir: string, current: string] {
    let sidecars = (
        ls $symbols_dir
        | where name =~ '\.debug\.wasm$'
        | sort-by modified --reverse
    )
    if ($sidecars | length) <= $KEEP {
        return
    }
    for stale in ($sidecars | skip $KEEP | where name != $current) {
        rm $stale.name
        print $"pruned   ($stale.name)  ($stale.size)"
    }
}

# The wasm this build actually serves, followed from index.html.
#
# NOT the newest file matching a glob: dx never removes superseded assets, so
# that directory accumulates wasm files from every previous build and the wrong
# one would be split — leaving the shipped binary fat and the sidecar useless.
def current_wasm [] {
    let entry = (
        open $"($PUBLIC)/index.html"
        | parse --regex 'wiki-dioxus-(?<h>[0-9a-z]+)\.js'
        | get h.0
    )
    let name = (
        open $"($PUBLIC)/assets/wiki-dioxus-($entry).js"
        | parse --regex 'wiki-dioxus_bg-(?<h>[0-9a-z]+)\.wasm'
        | get h.0
    )
    { hash: $name, path: $"($PUBLIC)/assets/wiki-dioxus_bg-($name).wasm" }
}

def main [] {
    let target = (current_wasm)
    if not ($target.path | path exists) {
        print $"referenced wasm missing: ($target.path)"
        exit 1
    }

    let symbols_dir = $"($PUBLIC)/symbols"
    mkdir $symbols_dir
    let sidecar = $"($symbols_dir)/($target.hash).debug.wasm"

    # This rewrites the build output in place, and dx reuses an unchanged wasm
    # rather than regenerating it — so a second `just build` finds the binary
    # already stripped. That is success, not failure, as long as the sidecar from
    # the first run is still there. Without this check the split would overwrite
    # a good sidecar with a copy of the stripped binary, quietly destroying the
    # symbols it exists to keep.
    let has_debug = (^wasm-tools objdump $target.path | str contains ".debug_")
    if not $has_debug {
        if ($sidecar | path exists) {
            print $"already split  ($sidecar)"
            prune_symbols $symbols_dir $sidecar
            return
        }
        print "shipped wasm has no debug sections and no sidecar exists —"
        print "was it built without --debug-symbols?"
        exit 1
    }

    cp $target.path $sidecar

    # `-d` takes a regex over section names. The DWARF sections are the only ones
    # worth megabytes; everything else stays, so nothing else about the module
    # changes.
    ^wasm-tools strip -d '^\.debug_' $sidecar -o $target.path

    # Optimise here, not in dx. dx runs wasm-opt BEFORE this script, while the
    # DWARF is still in the module, and binaryen aborts on it (SIGABRT), so the
    # optimisation was silently skipped and the unoptimised binary shipped. On
    # the stripped module it takes about two seconds and gives back a good tenth
    # of the payload. Rewriting the bytes under dx's content-hashed filename is
    # what the strip above already does; nothing revalidates the hash.
    let optimised = $"($target.path).opt"
    ^wasm-opt -Oz $target.path -o $optimised
    mv --force $optimised $target.path

    let symbols_size = (ls $sidecar | get size.0)
    let shipped_size = (ls $target.path | get size.0)
    print $"symbols  ($sidecar)  ($symbols_size)"
    print $"shipped  ($target.path)  ($shipped_size)"
    if $shipped_size >= $symbols_size {
        print "WARNING: stripping freed nothing — was the build made without debug info?"
        exit 1
    }

    prune_symbols $symbols_dir $sidecar
}
