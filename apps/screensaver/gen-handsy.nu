#!/usr/bin/env nu

# Convert handsy's key-frame animations into a Rust table.
#
# `hacks/glx/handsy_anim.h` is upstream's hand poses and the animations that
# string them together, written by hand with the saver's own debug mode. It is
# C: nested array initialisers, a few macros that paste a shake or a round of
# rock-paper-scissors into place, and arithmetic on M_PI. All of that is
# expanded here rather than transcribed by hand.
#
# Usage:
#   nu gen-handsy.nu <handsy_anim.h> <out-file>

# A C number becomes a Rust float, and M_PI becomes the constant.
def numbers [s: string] {
    $s
    | str replace --all --regex '\bM_PI\b' 'PI'
    | str replace --all --regex '\bTrue\b' 'true'
    | str replace --all --regex '\bFalse\b' 'false'
    | str replace --all --regex '(?<![\w.])(\d+)(?![\w.])' '$1.0'
}

# `&open_palm` is a reference to a pose, and `tap_anim` to an animation.
def rustname [s: string] {
    $s | str upcase
}

def main [header: string, out_file: string] {
    let raw = (open $header --raw | decode utf-8)

    # Comments first: they hold braces and commas of their own.
    mut text = ($raw | str replace --all --regex '(?s)/\*.*?\*/' " ")

    # Join the continued lines of the multi-line macros, then expand every
    # macro, longest name first so that RPS_SHAKE is not eaten by RPS_S.
    $text = ($text | str replace --all --regex '\\\s*\n' " ")
    # The separator has to be a space or a tab rather than any whitespace:
    # a define with no body would otherwise swallow the following line.
    let defines = ($text
        | parse --regex '(?m)^#define[ \t]+(?<name>\w+)[ \t]+(?<body>[^\n]*)'
        | sort-by {|d| ($d.name | str length) } --reverse)
    # Twice over, because one macro pastes another in.
    for pass in 0..2 {
        for d in $defines {
            $text = ($text | str replace --all $d.name $d.body)
        }
    }

    # The poses.
    let poses = ($text
        | parse --regex '(?s)static const hand (?<name>\w+) = \{(?<body>.*?)\};')
    mut out = "//! The hand poses and animations of `hacks/glx/handsy_anim.h`,
//! converted by `gen-handsy.nu`.
//!
//! ```text
//! handsy, Copyright (c) 2018 Jamie Zawinski <jwz@jwz.org>
//!
//! Handsy's various animation key-frames.  I made these by \"hand\" with -debug.
//!
//! I considered using the Leap Motion Controller API to snapshot real hands,
//! but that device is crap at detecting poses with any precision.
//! ```
//!
//! A pose is how far every joint is bent; an animation is a list of poses with
//! how long to take getting to each and how long to wait there, and where the
//! whole hand is while it does. The pairs at the end say which animation each
//! hand runs, since some of them are two hands doing different things.

use std::f64::consts::PI;

/// How far each joint is bent, and where the hand is.
#[derive(Clone, Copy)]
pub struct Hand {
    /// Five digits, four bones each, thumb first.
    pub joint: [[f64; 4]; 5],
    /// How far each digit is spread from the next.
    pub base: [f64; 5],
    /// Up and down, side to side, and the twist.
    pub wrist: [f64; 3],
    pub pos: [f64; 3],
    /// Whether it is a left hand.
    pub sinister: bool,
}

/// One key frame: the pose to reach, how long to take, how long to wait, and
/// where the hand is while it does.
pub struct HandAnim {
    pub dest: &'static Hand,
    pub duration: f64,
    pub pause: f64,
    pub pos: [f64; 3],
    pub rot: [f64; 3],
}

/// What the two hands do, and how far behind the left the right one is.
pub struct HandAnimPair {
    pub pair: [&'static [HandAnim]; 2],
    pub delay: f64,
}
"

    for p in $poses {
        let fields = ($p.body
            | parse --regex '(?s)\{(?<rows>.*)\}\s*,\s*\{(?<base>[^{}]*)\}\s*,\s*\{(?<wrist>[^{}]*)\}\s*,\s*\{(?<pos>[^{}]*)\}\s*,\s*(?<sin>\w+)'
            | first)
        let rows = ($fields.rows
            | parse --regex '\{(?<r>[^{}]*)\}'
            | each {|r| "[" + (numbers ($r.r | str trim | str trim --char ',')) + "]" }
            | str join ", ")
        $out = $out + $"
pub static (rustname $p.name): Hand = Hand \{
    joint: [($rows)],
    base: [(numbers ($fields.base | str trim))],
    wrist: [(numbers ($fields.wrist | str trim))],
    pos: [(numbers ($fields.pos | str trim))],
    sinister: (numbers $fields.sin),
\};
"
    }

    # The animations. The last row of every one is the `{ 0, }` terminator.
    let anims = ($text
        | parse --regex '(?s)static const hand_anim (?<name>\w+)\[\] = \{(?<body>.*?)\};')
    for a in $anims {
        let rows = ($a.body
            | parse --regex '\{\s*&(?<dest>\w+)\s*,\s*(?<dur>[^,]+),\s*(?<pause>[^,]+),\s*\{(?<pos>[^{}]*)\}\s*,\s*\{(?<rot>[^{}]*)\}\s*\}')
        mut body = ""
        for r in $rows {
            $body = $body + $"    HandAnim \{ dest: &(rustname $r.dest), duration: (numbers ($r.dur | str trim)), pause: (numbers ($r.pause | str trim)), pos: [(numbers ($r.pos | str trim))], rot: [(numbers ($r.rot | str trim))] \},\n"
        }
        $out = $out + $"
pub static (rustname $a.name): &[HandAnim] = &[
($body)];
"
    }

    let pairs = ($text
        | parse --regex '(?s)static const hand_anim_pair all_hand_anims\[\] = \{(?<body>.*?)\};'
        | first | get body
        | parse --regex '\{\{\s*(?<a>\w+)\s*,\s*(?<b>\w+)\s*\}\s*,\s*(?<delay>[^}]+)\}')
    mut body = ""
    for p in $pairs {
        $body = $body + $"    HandAnimPair \{ pair: [(rustname $p.a), (rustname $p.b)], delay: (numbers ($p.delay | str trim)) \},\n"
    }
    $out = $out + $"
/// Every animation, and which hand runs which.
pub static ALL_HAND_ANIMS: &[HandAnimPair] = &[
($body)];
"

    $out | save -f $out_file
    print $"wrote ($out_file): ($poses | length) poses, ($anims | length) animations, ($pairs | length) pairs"
}
