#!/usr/bin/env nu

# Convert XScreenSaver's Lightwave objects into loadable assets.
#
# `pipes` draws its valves, gauges and bolts from nine models that Ed Mackey
# converted out of Lightwave in 1997. They arrive as three flat C arrays each,
# a hundred kilobytes of them in `hacks/glx/pipeobjs.c`: the points, one
# normal per polygon, and a stream of polygon records. A record is a count,
# that many point indices, and a filler slot; a count of nought ends the
# stream. `runtime::lwo` walks exactly that, so the arrays are converted here
# rather than rewritten.
#
# The output keeps upstream's literals character for character and only strips
# the C around them:
#
#     LWO1 <name> <num_pnts>
#     pnts <count>
#     <floats>
#     normals <count>
#     <floats>
#     pols <count>
#     <integers>
#
# Usage:
#
#     nu gen-lwo.nu <upstream-glx-dir> <out-dir> <objects.c>

# The literals of one `static const TYPE NAME[] = { ... };` array, verbatim.
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

# Numbers, twelve to a line, so that a converted model diffs readably.
def wrap [values: list<string>] {
    $values | chunks 12 | each {|c| $c | str join " " } | str join "\n"
}

def main [glx_dir: string, out_dir: string, source: string] {
    let path = ([$glx_dir, $source] | path join)
    let text = (open --raw $path | str replace --all --regex '(?s)/\*.*?\*/' ' ')

    let objects = ($text | parse --regex
        '(?s)struct lwo\s+LWO_(?<name>\w+)\s*=\s*\{\s*(?<num>\d+)\s*,\s*(?<pnts>\w+)\s*,\s*(?<normals>\w+)\s*,\s*(?<pols>\w+)')
    if ($objects | is-empty) { error make {msg: $"no lwo objects in ($path)"} }

    mut consts = []
    for o in $objects {
        let pnts = (read_array $text $o.pnts)
        let normals = (read_array $text $o.normals)
        let pols = (read_array $text $o.pols)
        let out = ([
            $"LWO1 ($o.name) ($o.num)"
            $"pnts (($pnts | length))"
            (wrap $pnts)
            $"normals (($normals | length))"
            (wrap $normals)
            $"pols (($pols | length))"
            (wrap $pols)
        ] | str join "\n")

        let slug = ($o.name | str lowercase)
        let file = ([$out_dir, $"pipes_($slug).lwo"] | path join)
        $out | save -f $file
        print $"($o.name): ($o.num) points, (($pols | length)) polygon words -> ($file)"
        let upper = ($o.name | str upcase)
        $consts = ($consts | append
            $"pub const PIPES_($upper): &str = include_str!(char lparen)\"../models/pipes_($slug).lwo\"(char rparen);")
    }
    print ""
    $consts | str join "\n" | print
}
