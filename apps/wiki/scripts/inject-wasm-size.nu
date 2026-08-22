#!/usr/bin/env nu

# Write the shipped wasm's byte count into the built index.html, and preload it.
#
# dx preloads the 17 KB glue script but not the 2 MB wasm, so the browser only
# learns about the big download once the glue has arrived and run: one extra
# round trip, on exactly the connection where round trips hurt (a hall full of
# phones on one access point). The filename is content-hashed, so the link has
# to be injected here rather than written into the source page.
#
# `as="fetch"` with `crossorigin` because that is what the glue's own request
# is: a plain `fetch()`, which is CORS mode with `same-origin` credentials. Get
# either half wrong and the preload does not match, so the wasm downloads TWICE
# and this makes things worse. Verified in a browser, counting requests.
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
    root: string = "target/dx/wiki/release/web/public"
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

    # Beside dx's own preload of the glue, so both start at once.
    let href = $"/./assets/($wasms | first)"
    let preload = $"<link rel=\"preload\" as=\"fetch\" type=\"application/wasm\" href=\"($href)\" crossorigin>"
    if not ($html | str contains "</head>") {
        print $"($index) has no </head> to inject the preload before"
        exit 1
    }
    let with_preload = if ($html | str contains $href) and ($html | str contains "rel=\"preload\" as=\"fetch\"") {
        print "wasm preload already present"
        $html
    } else {
        $html | str replace "</head>" $"($preload)</head>"
    }

    (
        $with_preload
        | str replace $marker $"window.__WASM_BYTES__ = ($bytes);"
        | save --force --raw $index
    )
    print $"wasm size  ($bytes) bytes  ($wasms | first)  -> ($index)"
    print $"wasm preload  ($href)"
}
