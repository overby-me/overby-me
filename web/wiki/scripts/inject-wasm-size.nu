#!/usr/bin/env nu

# Write the shipped wasm's byte count into the built index.html.
#
# The boot screen shows download progress, and it needs a denominator it cannot
# get from the network: the bundle is served gzipped and chunked, so there is no
# `Content-Length`, and a stream reader counts DECOMPRESSED bytes, which that
# header would not describe even when it is sent. The size IS known here, right
# after the build produced the file.
#
# Run after `dx build` and after split-symbols.nu, so the size is the one a
# reader actually downloads (symbols already moved out).
#
# The wasm is found by following what the page ACTUALLY loads --
# index.html names a glue script, the glue script names the wasm -- rather than
# by globbing assets/. Old builds are not cleaned out of that directory, so a
# glob would go ambiguous on the second build in a row and there would be no
# honest way to choose between the hits.

# Every `/assets/…` path of one extension that a file refers to.
def refs [text: string, pattern: string] {
    $text | parse --regex $pattern | get file | uniq
}

def main [
    root: string = "target/dx/wiki-dioxus/release/web/public"
] {
    let index = ($root | path join "index.html")
    if not ($index | path exists) {
        print $"($index) does not exist; nothing to inject"
        exit 1
    }
    let html = (open --raw $index)

    let scripts = (refs $html "/assets/(?<file>[^\"' )>]+\\.js)")
    let wasms = (
        $scripts
        | each {|js|
            let path = ($root | path join "assets" | path join $js)
            if ($path | path exists) {
                refs (open --raw $path) "/assets/(?<file>[^\"' )>]+\\.wasm)"
            } else {
                []
            }
        }
        | flatten
        | uniq
    )

    if ($wasms | length) != 1 {
        print $"($index) loads ($wasms | length) wasm files; expected one"
        print ($wasms | str join "\n")
        exit 1
    }
    let wasm = ($root | path join "assets" | path join ($wasms | first))
    if not ($wasm | path exists) {
        print $"($wasm) is referenced by the bundle but is not on disk"
        exit 1
    }

    let bytes = (ls $wasm | first | get size | into int)
    let marker = "window.__WASM_BYTES__ = 0;"
    if not ($html | str contains $marker) {
        print $"($index) has no ($marker) to replace"
        exit 1
    }

    (
        $html
        | str replace $marker $"window.__WASM_BYTES__ = ($bytes);"
        | save --force --raw $index
    )
    print $"wasm size  ($bytes) bytes  ($wasms | first)  -> ($index)"
}
