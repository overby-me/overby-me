#!/usr/bin/env nu

# Extract tangram's `solved[]` table from upstream's C into a Rust const.
#
# Each entry is a puzzle name and seven shape placements. Everything but the
# position, the two rotations and the direction is the same constant in every
# row, so only those four are carried over.

# Point this at an upstream checkout.
const SRC = "../../xscreensaver/hacks/glx/tangram.c"

let text = (open $SRC --raw | decode utf-8)
let start = ($text | str index-of "static const puzzle solved[] = {")
let body = ($text | str substring $start..)
let stop = ($body | str index-of "
};")
let table = ($body | str substring ..<$stop)

# `{"Name", {` opens a puzzle.
let names = ($table | parse --regex '\{"(?<name>[^"]+)",' | get name)

# Each placement: {{x, y, z}, r, fr, dl, INIT_DZ, -SPEED, 0, Up},
# Upstream line 593 omits the `dl` field, so C shifts the rest along and the
# row ends up with up = false. The optional group matches it either way.
let rows = ($table | parse --regex '\{\{(?<x>[-0-9.]+),\s*(?<y>[-0-9.]+),\s*(?<z>[-0-9.]+)\},\s*(?<r>[-0-9]+),\s*(?<fr>[-0-9]+),\s*(?:(?<dl>[-0-9]+),\s*)?INIT_DZ,\s*-SPEED,\s*0,(?:\s*(?<up>True|False))?\s*\}')

print $"puzzles: ($names | length), placements: ($rows | length)"
if ($rows | length) != (($names | length) * 7) {
    error make {msg: "the table is not seven placements a puzzle"}
}
let zs = ($rows | get z | uniq)
if $zs != ["0"] {
    print $"note: z is not always zero: ($zs)"
}

# The table writes every coordinate to six decimals, which is more digits
# than an f32 holds; trim the trailing zeros so the literals are exact.
def trim [v: string] {
    if ($v | str contains ".") {
        let t = ($v | str trim --right --char "0")
        if ($t | str ends-with ".") { $t + "0" } else { $t }
    } else { $v }
}

let out = ($names | enumerate | each {|it|
    let seven = ($rows | skip ($it.index * 7) | first 7 | each {|p|
        $"        Placed { x: (trim $p.x), y: (trim $p.y), r: ($p.r), fr: ($p.fr), up: (if $p.up == "" { "false" } else { $p.up | str downcase }) },"
    } | str join "\n")
    $"    Solution {
        name: \"($it.item)\",
        shapes: [
($seven)
        ],
    },"
} | str join "\n")

const HEADER = "// The forty-five figures, as the finished position of each of the seven
// pieces. Extracted from upstream `tangram.c`'s `solved[]` table by
// `web/homepage/gen-tangram.nu`; the fields that are the same constant in every
// row of it are left out.

"

$HEADER + "const SOLUTIONS: &[Solution] = &[\n" + $out + "\n];\n"
| save -f xscreensaver/src/hacks3d/tangram_solutions.rs
print "wrote xscreensaver/src/hacks3d/tangram_solutions.rs"
