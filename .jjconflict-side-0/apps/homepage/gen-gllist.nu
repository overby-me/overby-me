#!/usr/bin/env nu

# Convert XScreenSaver's generated model files into loadable assets.
#
# Upstream ships its 3D models as C source: a flat `static const float data[]`
# of interleaved vertex attributes, plus a `struct gllist` header saying how to
# read it. That is fine for a C program, where the compiler turns the literals
# back into bytes, but a Rust file with tens of thousands of float literals in
# it takes minutes to compile, so the models are converted once, here, and the
# ported savers `include_str!` the result.
#
# The output keeps upstream's literals character for character and only strips
# the C around them, so a converted model diffs cleanly against its source. It
# is one header line and then the numbers:
#
#     GLL1 <format> <primitive> <points>
#     <float> <float> ...
#
# where <format> is v3f, c3f_v3f or n3f_v3f and <primitive> is points, lines,
# triangles or quads. Vertices carry three floats under v3f and six otherwise.
#
# Usage:
#
#     nu gen-gllist.nu <upstream-glx-dir> <out-dir> <model.c>...
#
# where <upstream-glx-dir> is a checkout's hacks/glx. One `.gllist` is written
# per object the file defines (a single .c may define several), and the Rust
# `const` lines to paste into xscreensaver/src/models.rs are printed at the end.

const FORMATS = {GL_V3F: "v3f", GL_C3F_V3F: "c3f_v3f", GL_N3F_V3F: "n3f_v3f"}
const PRIMITIVES = {
    GL_POINTS: "points",
    GL_LINES: "lines",
    GL_TRIANGLES: "triangles",
    GL_QUADS: "quads",
}

# The literals of one `static const float NAME[] = { ... };` array, verbatim.
def read_array [text: string, name: string] {
    let start = ($text | str index-of ($name + "[]"))
    if $start < 0 { error make {msg: $"no array named ($name)"} }
    let open = ($text | str substring $start.. | str index-of "{")
    let body = ($text | str substring ($start + $open + 1)..)
    let close = ($body | str index-of "}")

    $body | str substring ..<$close
    | split row ","
    | each {|s| $s | str trim }
    | where {|s| $s != "" }
}

def main [glx_dir: string, out_dir: string, ...files: string] {
    mkdir $out_dir
    mut consts = []

    for file in $files {
        let path = if ($file | path exists) { $file } else { $glx_dir | path join $file }
        # Comments hold commas and braces of their own, so they go first.
        let text = (open $path --raw | decode utf-8 | str replace --all --regex '(?s)/\*.*?\*/' " ")

        # `static const struct gllist NAME = { FORMAT, PRIM, POINTS, DATA, NEXT };`
        let frames = ($text | parse --regex '(?s)struct gllist\s+(?<frame>\w+)\s*=\s*\{\s*(?<format>GL_\w+)\s*,\s*(?<prim>GL_\w+)\s*,\s*(?<points>\d+)\s*,\s*(?<data>\w+)\s*,\s*(?<next>\w+)\s*\}')

        for f in $frames {
            if $f.next != "NULL" and $f.next != "0" {
                error make {msg: $"($path): frame ($f.frame) chains to ($f.next), unsupported"}
            }

            # `const struct gllist *NAME = &FRAME;` gives the exported name.
            let exported = ($text
                | parse --regex ('struct gllist \*(?<name>\w+)\s*=\s*&' + $f.frame + '\s*;')
                | get -o name.0 | default $f.frame)

            let format = ($FORMATS | get -o $f.format)
            let prim = ($PRIMITIVES | get -o $f.prim)
            if $format == null { error make {msg: $"($path): unknown format ($f.format)"} }
            if $prim == null { error make {msg: $"($path): unknown primitive ($f.prim)"} }

            let points = ($f.points | into int)
            let stride = (if $format == "v3f" { 3 } else { 6 })
            let floats = (read_array $text $f.data)
            if ($floats | length) != ($points * $stride) {
                error make {msg: $"($path): ($exported) has ($floats | length) floats, expected ($points * $stride)"}
            }

            let out = ($out_dir | path join $"($exported).gllist")
            let rows = ($floats | chunks $stride | each {|c| $c | str join " " })
            ([$"GLL1 ($format) ($prim) ($points)"] | append $rows | str join "\n") + "\n" | save -f $out

            let name = ($exported | str uppercase)
            $consts = ($consts | append (
                'pub const ' + $name + ': &str = include_str!("../models/' + $exported + '.gllist");'))
            print $"($exported): ($points) verts, ($format), ($prim) -> ($out)"
        }
    }

    print ""
    print ($consts | str join "\n")
}
