#!/usr/bin/env nu

# Convert XScreenSaver's `curlicue.h` into a raw greyscale asset.
#
# The four topology savers by Carsten Steger can draw a curling arrow over
# their surface to show its orientation, which is how you see that a
# projective plane is non-orientable: walk around it and the arrow comes back
# the other way. Upstream ships that arrow as a 64x64 array of bytes in a
# header; this writes the same bytes out as a file the ports `include_bytes!`.
#
# Usage:
#
#     nu gen-curlicue.nu <upstream-glx-dir>
#
# where <upstream-glx-dir> is a checkout's hacks/glx.

def main [glx_dir: string] {
    let src = ($glx_dir | path join "curlicue.h")
    let text = (open $src --raw | decode utf-8)

    let dim = ($text | parse --regex '#define TEX_DIMENSION\s+(?<n>\d+)' | get n.0 | into int)

    let open_brace = ($text | str index-of "{")
    let close_brace = ($text | str index-of "};")
    let body = ($text | str substring ($open_brace + 1)..<$close_brace)

    let bytes = ($body
        | split row ","
        | each {|s| $s | str trim }
        | where {|s| $s != "" }
        | each {|s| $s | into int })

    if ($bytes | length) != ($dim * $dim) {
        error make {msg: $"got ($bytes | length) bytes, expected ($dim * $dim)"}
    }
    if ($bytes | any {|b| $b < 0 or $b > 255 }) {
        error make {msg: "a value is not a byte"}
    }

    mkdir xscreensaver/images
    let out = "xscreensaver/images/curlicue.gray"
    # `into binary` on an integer gives eight little-endian bytes, so the low
    # one is byte zero; taking it is exact for anything in 0..255.
    ($bytes | each {|b| $b | into binary | bytes at 0..<1 } | bytes collect) | save -f $out
    print $"($dim)x($dim) greyscale -> ($out)"
    print ""
    print 'pub const CURLICUE: &[u8] = include_bytes!("../images/curlicue.gray");'
}
