#!/usr/bin/env nu

# Convert XScreenSaver's Hopf fibration animation tables into a loadable asset.
#
# `hacks/glx/hopfanimations.c` is half a megabyte of nested C struct literals
# behind a layer of token-pasting macros: 189 arrays of per-object animations,
# 189 records that group them, 134 lists of phases, 65 sets of animations, and
# an 8x8 table saying which set takes the fibration from one configuration to
# another. Compiling that as Rust source would be slow and unreadable, so it is
# converted once, here, and read back by `runtime::hopfanimations`.
#
# The output keeps upstream's numbers, evaluated: the file writes angles as
# expressions like `3.0f*M_PI_F/4.0f`, and a float is a float. Names are kept
# as upstream spells them so the asset diffs against its source.
#
#     HOPF1
#     so <name> <count>
#     <24 numbers>                   (one line per animated object)
#     mo <name> <so> <prob> <easing> <ax> <ay> <az> <a0> <a1> <easing> <steps>
#     ps <name> <mo>...
#     anims <name> <ps>...
#     table <anims>...               (64 names, row-major)
#
# Usage:
#
#     nu gen-hopfanimations.nu <upstream-glx-dir> <out-file>

# The symbolic constants the tables are written in terms of. Everything else in
# them is a number or one of `*` and `/`.
const CONSTANTS = {
    M_PI_F: "3.1415926535898",
    GEN_TORUS: "0",
    GEN_SPIRAL: "1",
    EASING_NONE: "0",
    EASING_CUBIC: "1",
    EASING_SIN: "2",
    EASING_COS: "3",
    EASING_LIN: "4",
    EASING_ACCEL: "5",
    EASING_DECEL: "6",
}

# How many numbers one `animation_single_obj` is worth: a generator, five
# animated parameters as start, end and easing function, a wave number and a
# count, a rotation axis, and a rotation as start, end and easing function.
const SO_FIELDS = 24

# Evaluate one C float expression. They are all a number or a product and
# quotient of numbers, with an optional leading minus.
def evalf [expr: string] {
    let toks = ($expr | str trim
        | str replace --all "*" " * "
        | str replace --all "/" " / "
        | split row --regex '\s+'
        | where {|t| $t != "" })
    mut val = ($toks | first | into float)
    mut i = 1
    while $i < ($toks | length) {
        let op = ($toks | get $i)
        let n = ($toks | get ($i + 1) | into float)
        $val = (if $op == "*" { $val * $n } else { $val / $n })
        $i = $i + 2
    }
    $val
}

# Format a float for the asset. Rust reads these back with `parse::<f32>`, so
# whatever nushell prints for a float is what it wants.
def fmtf [v: float] {
    $"($v)"
}

# Every `MACRO(name) = { ... };` block in the text, as a record of the macro
# name, the block name, and the initializer with its braces taken out.
def blocks [text: string] {
    $text
    | split row "\n};"
    | each {|chunk|
        let m = ($chunk | parse --regex '(?s)(?<macro>ANIM[A-Z_]*_DEF)\((?<name>\w+)\)\s*=\s*\{(?<body>.*)$')
        if ($m | is-empty) { null } else { $m | first }
    }
    | compact
}

# The comma-separated items of an initializer, with nested braces flattened
# away: every one of these structures is a flat list of scalars once the
# grouping is dropped.
def items [body: string] {
    $body
    | str replace --all "{" ""
    | str replace --all "}" ""
    | split row ","
    | each {|s| $s | str trim }
    | where {|s| $s != "" }
}

def main [glx_dir: string, out_file: string] {
    let src = ([$glx_dir, "hopfanimations.c"] | path join)
    mut text = (open --raw $src)

    # Comments carry the field names and nothing a reader of the asset needs.
    $text = ($text | str replace --all --regex '(?s)/\*.*?\*/' ' ')

    # Substitute the constants, longest name first so that no name is a prefix
    # of another one's replacement.
    for name in ($CONSTANTS | columns | sort-by --custom {|a, b| ($a | str length) > ($b | str length) }) {
        $text = ($text | str replace --all $name ($CONSTANTS | get $name))
    }
    # Float literals are written with a trailing f.
    $text = ($text | str replace --all --regex '([0-9.])f\b' '${1}')

    let defs = (blocks $text)
    mut out = ["HOPF1"]
    mut n_so = 0
    mut n_mo = 0
    mut n_ps = 0
    mut n_anims = 0

    for d in $defs {
        let it = (items $d.body)
        match $d.macro {
            "ANIM_SO_DEF" => {
                let n = (($it | length) / $SO_FIELDS | into int)
                if ($it | length) != ($n * $SO_FIELDS) {
                    error make {msg: $"($d.name): ($it | length) fields is not a multiple of ($SO_FIELDS)"}
                }
                $out = ($out | append $"so ($d.name) ($n)")
                for i in 0..<$n {
                    let row = ($it | skip ($i * $SO_FIELDS) | first $SO_FIELDS
                        | each {|e| fmtf (evalf $e) })
                    $out = ($out | append ($row | str join " "))
                }
                $n_so = $n_so + 1
            }
            "ANIM_MO_DEF" => {
                # count, single-object array, then nine scalars.
                let so = ($it | get 1 | parse --regex 'ANIM_SO_NAME\((?<n>\w+)\)' | get n.0)
                let rest = ($it | skip 2 | each {|e| fmtf (evalf $e) })
                $out = ($out | append $"mo ($d.name) ($so) ($rest | str join ' ')")
                $n_mo = $n_mo + 1
            }
            "ANIM_PH_DEF" => {
                let mos = ($it | each {|e| $e | parse --regex 'ANIM_MO_REF\((?<n>\w+)\)' | get n.0 })
                $out = ($out | append $"ps ($d.name) ($mos | str join ' ')")
                $n_ps = $n_ps + 1
            }
            "ANIM_PS_DEF" => { }
            "ANIMS_M_DEF" => {
                let pss = ($it | each {|e| $e | parse --regex 'ANIM_PS_REF\((?<n>\w+)\)' | get n.0 })
                $out = ($out | append $"anims ($d.name) ($pss | str join ' ')")
                $n_anims = $n_anims + 1
            }
            "ANIMS_DEF" => { }
            _ => { error make {msg: $"unknown macro ($d.macro)"} }
        }
    }

    # The 8x8 table of which set of animations goes from one configuration of
    # the fibration to another.
    let table = ($text
        | parse --regex '(?s)hopf_animations\[[^\]]*\]\[[^\]]*\]\s*=\s*\{(?<body>.*?)\n\};'
        | get body.0
        | parse --regex 'ANIMS_REF\((?<n>\w+)\)'
        | get n)
    if ($table | length) != 64 {
        error make {msg: $"the table has ($table | length) entries, not 64"}
    }
    $out = ($out | append $"table ($table | str join ' ')")

    $out | str join "\n" | save -f $out_file
    print $"($n_so) object sets, ($n_mo) animations, ($n_ps) phase lists, ($n_anims) sets -> ($out_file)"
}
