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
    cp $target.path $sidecar

    # `-d` takes a regex over section names. The DWARF sections are the only ones
    # worth megabytes; everything else stays, so nothing else about the module
    # changes.
    ^wasm-tools strip -d '^\.debug_' $sidecar -o $target.path

    let symbols_size = (ls $sidecar | get size.0)
    let shipped_size = (ls $target.path | get size.0)
    print $"symbols  ($sidecar)  ($symbols_size)"
    print $"shipped  ($target.path)  ($shipped_size)"
    if $shipped_size >= $symbols_size {
        print "WARNING: stripping freed nothing — was the build made without debug info?"
        exit 1
    }
}
