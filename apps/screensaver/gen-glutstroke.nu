#!/usr/bin/env nu

# Convert GLUT's stroke fonts into a Rust table.
#
# `hacks/glx/glut_roman.h` and `glut_mroman.h` are vector fonts: each character
# is a handful of open polylines in a hundred-unit em, plus how far to move
# along afterwards. They are C source, one `static const CoordRec` array per
# stroke and a table gathering them, which is fine for a C program but is tens
# of thousands of float literals for rustc to chew on, so they are converted
# once, here.
#
# Usage:
#   nu gen-glutstroke.nu <glx-dir> <out-file>

# One `charNN_strokeM` array (the monospaced font calls them `monoCharNN`): the points of a single polyline.
def read_strokes [text: string] {
    $text
    | parse --regex '(?s)static const CoordRec (?<name>\w+_stroke\d+)\[\] = \{(?<body>.*?)\};'
    | each {|s|
        let pts = ($s.body
            | parse --regex '\{\s*(?<x>[-0-9.e]+)\s*,\s*(?<y>[-0-9.e]+)\s*\}'
            | each {|p|
                # Every number is written as a float: a bare 100 is an integer
                # to rustc.
                let x = (if ($p.x | str contains ".") { $p.x } else { $p.x + ".0" })
                let y = (if ($p.y | str contains ".") { $p.y } else { $p.y + ".0" })
                $"[($x), ($y)]"
            }
            | str join ", ")
        {name: $s.name, pts: $pts}
    }
}

# The `charNN` tables say which strokes make up a character, and the `chars`
# table says how many strokes each character has and how wide it is.
def read_chars [text: string] {
    $text
    | parse --regex '(?s)static const StrokeRec (?<name>\w+)\[\] = \{(?<body>.*?)\};'
    | each {|c|
        let strokes = ($c.body
            | parse --regex '\{\s*\d+\s*,\s*(?<s>\w+_stroke\d+)\s*\}'
            | get s)
        {name: $c.name, strokes: $strokes}
    }
}

def main [glx_dir: string, out_file: string] {
    mut out = "//! The GLUT stroke fonts, converted from `hacks/glx/glut_roman.h`
//! and `hacks/glx/glut_mroman.h` by `gen-glutstroke.nu`.
//!
//! ```text
//! Roman simplex stroke font copyright (c) 1989, 1990, 1991
//! by Sun Microsystems, Inc. and the X Consortium.
//! Originally part of the GLUT library by Mark J. Kilgard.
//! ```
//!
//! A character is a few open polylines in a hundred-unit em and how far to
//! move along after drawing it. There are no closed shapes and no fills: a
//! saver that wants a solid letter draws something along the lines itself,
//! which is what `gltext` does with tubes and spheres.

/// One character: its polylines, where its middle is, and how far along to
/// move afterwards.
pub struct StrokeChar {
    pub strokes: &'static [&'static [[f32; 2]]],
    pub center: f32,
    pub right: f32,
}

/// A whole font: its characters and the top and bottom of its em.
pub struct StrokeFont {
    pub chars: &'static [StrokeChar],
    pub top: f32,
    pub bottom: f32,
}
"

    for font in [
        {file: "glut_roman.h",  var: "ROMAN",      rec: "glutStrokeRoman",     table: "chars"},
        {file: "glut_mroman.h", var: "MONO_ROMAN", rec: "glutStrokeMonoRoman", table: "monoChars"},
    ] {
        let path = ([$glx_dir $font.file] | path join)
        # The comments hold the character names and would break the parse.
        let text = (open $path --raw | decode utf-8
            | str replace --all --regex '(?s)/\*.*?\*/' " ")

        let strokes = (read_strokes $text)
        let chars = (read_chars $text)
        let by_name = ($strokes | reduce --fold {} {|s, acc| $acc | insert $s.name $s.pts })
        let char_strokes = ($chars | reduce --fold {} {|c, acc| $acc | insert $c.name $c.strokes })

        # The `chars` table: how many strokes, which table, the middle, the
        # advance. A character with no strokes at all names a null table.
        let table_re = ('(?s)static const StrokeCharRec ' + $font.table
            + '\[\] = \{(?<body>.*?)\};')
        let table = ($text | parse --regex $table_re | first | get body)
        let rows = ($table
            | parse --regex '\{\s*(?<n>\d+)\s*,\s*(?<name>[a-zA-Z0-9_]+)\s*,\s*(?<center>[-0-9.]+)\s*,\s*(?<right>[-0-9.]+)\s*\}')

        # Built by concatenation: a regex is full of parentheses, and in an
        # interpolated string nushell reads those as commands to run.
        let re = ('StrokeFontRec ' + $font.rec
            + ' = \{ "[^"]*", \d+, \w+, (?<top>[-0-9.]+), (?<bottom>[-0-9.]+) \}')
        let font_rec = ($text | parse --regex $re | first)

        mut body = ""
        for row in $rows {
            let names = (if ($row.name == "NULL") { [] } else { ($char_strokes | get -o $row.name | default []) })
            let strokes_src = ($names | each {|n| "&[" + ($by_name | get $n) + "]" } | str join ", ")
            # Every number is written as a float: a bare 0 is an integer to
            # rustc, and these fields are all f32.
            let c = (if ($row.center | str contains ".") { $row.center } else { $row.center + ".0" })
            let r = (if ($row.right | str contains ".") { $row.right } else { $row.right + ".0" })
            $body = $body + $"    StrokeChar { strokes: &[($strokes_src)], center: ($c), right: ($r) },\n"
        }
        $out = $out + $"
/// `($font.rec)`, ($rows | length) characters.
pub const ($font.var): StrokeFont = StrokeFont {
    chars: &[
($body)    ],
    top: ($font_rec.top),
    bottom: ($font_rec.bottom),
};
"
    }

    $out | save -f $out_file
    print $"wrote ($out_file)"
}
