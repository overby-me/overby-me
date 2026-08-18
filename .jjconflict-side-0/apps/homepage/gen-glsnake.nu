#!/usr/bin/env nu

# Extract glsnake's shape table from upstream's C.
#
# A Rubik's Snake is twenty-four prisms hinged in a line, and a shape is
# nothing more than the angle of each of the twenty-three joints. Upstream
# writes those as the tokens ZERO, LEFT, PIN and RIGHT, which are 0, 90, 180
# and 270 degrees, and it ships nearly three hundred named shapes that way in
# a table a thousand lines long.
#
# One shape becomes one string of twenty-four letters here, each the first
# letter of upstream's token, so the table stays diffable against its source
# while taking a line apiece instead of five.
#
# Usage:
#
#     nu gen-glsnake.nu <upstream-glx-dir>
#
# which writes xscreensaver/src/hacks3d/glsnake_models.rs.

const LETTERS = {ZERO: "Z", LEFT: "L", PIN: "P", RIGHT: "R"}

def main [glx_dir: string] {
    let src = ($glx_dir | path join "glsnake.c")
    let text = (open $src --raw | decode utf-8)

    # The table runs from `static const struct model_s model[] = {` to the
    # closing brace of the array.
    let start = ($text | str index-of "static const struct model_s model[] = {")
    if $start < 0 { error make {msg: "no model table"} }
    let rest = ($text | str substring ($start)..)
    let end = ($rest | str index-of "\n};")
    let body = ($rest | str substring 0..<$end)

    # Every entry is `{ "name", { TOKEN, TOKEN, ... } }`, with the tokens
    # wrapped over several lines and #defines scattered between entries.
    let entries = ($body
        # One entry may carry a comment between its name and its joints.
        | parse --regex '\{\s*"(?<name>[^"]+)",\s*(?:/\*[^*]*\*/\s*)?\{(?<joints>[^}]*)\}'
        | each {|e|
            let joints = ($e.joints
                | split row ","
                | each {|s| $s | str trim }
                | where {|s| $s != "" }
                | each {|s|
                    let letter = ($LETTERS | get -o $s)
                    if $letter == null {
                        error make {msg: $"unknown joint value ($s) in ($e.name)"}
                    }
                    $letter
                })
            # A few entries list fewer than twenty-four values and let C zero
            # the rest of the array, which is a straight joint.
            if ($joints | length) > 24 {
                error make {msg: $"($e.name) has ($joints | length) joints"}
            }
            let padded = ($joints | append (0..<(24 - ($joints | length)) | each { "Z" }))
            {name: $e.name, joints: ($padded | str join "")}
        })

    if ($entries | length) < 279 {
        error make {msg: $"only found ($entries | length) shapes"}
    }

    # A parenthesis cannot be escaped inside an interpolated string, so the
    # tuples are built by concatenation.
    let open = "("
    let close = ")"
    let rows = ($entries
        | each {|e| "    " + $open + $'"($e.name)", "($e.joints)"' + $close + "," }
        | str join "\n")
    let out = "xscreensaver/src/hacks3d/glsnake_models.rs"
    let header = $"//! The shapes `glsnake` folds itself into, from upstream's `glsnake.c`.
//!
//! A Rubik's Snake is twenty-four prisms hinged in a line, so a shape is just
//! the angle of each joint. Upstream writes those as ZERO, LEFT, PIN and
//! RIGHT, meaning nought, ninety, a hundred and eighty and two hundred and
//! seventy degrees; here each is the first letter of the same token, so a
//! shape is one string of twenty-four.
//!
//! Written by `apps/homepage/gen-glsnake.nu`. Do not edit.

/// Every shape upstream ships, in its order, which is the order they are
/// folded in when the sequence is not shuffled.
pub const MODELS: &[($open)&str, &str($close)] = &[
($rows)
];
"
    $header | save -f $out
    print $"($entries | length) shapes -> ($out)"
    print $"first: ($entries | first | get name), last: ($entries | last | get name)"
}
