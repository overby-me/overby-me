#!/usr/bin/env nu
# Generates the table of which positions of an intrinsic share one type.
#
#   nu intrinsic-overloads.nu <llvm-source-tree> <out.rs>
#
# `corpus/intrinsic-signatures.nu` records a position whose type is the same
# in every documented instantiation, and records nothing where it varies.
# What that throws away is the other half of the story: two positions whose
# types vary *together* are one overloaded type, and a call giving them
# different types is calling something that does not exist. `llvm.umax` is
# documented `i32, i32 -> i32` and `<4 x i32>, <4 x i32> -> <4 x i32>`, so
# the two arguments and the result are one type, and
# `call i8 @llvm.umax(i8 0, i16 1)` is the "invalid intrinsic signature"
# upstream reports.
#
# The conclusion is only drawn where the type actually varies, and only from
# an intrinsic documented more than once. Two positions that are `i1` in
# every instantiation are equal by being fixed rather than by being tied,
# which the type table already says; concluding an overload from that would
# be inventing a rule out of a single `declare` line.
#
# Position 0 is the result and the arguments follow it, because upstream ties
# a result to an argument as readily as two arguments to each other:
# `llvm.ctlz` returns what its first argument is and takes a fixed `i1`
# second.

# The mangling suffix an instantiation adds: `llvm.smax.v4i32` is `llvm.smax`.
def strip-mangling [name: string]: nothing -> string {
  mut parts = ($name | split row ".")
  while (($parts | length) > 2
    and (($parts | last) =~ '^(v[0-9].*|nxv[0-9].*|p[0-9]+|i[0-9]+|f[0-9]+|bf[0-9]+|f80|f128|ppcf128|isVoid|a[0-9].*)$')) {
    $parts = ($parts | drop 1)
  }
  $parts | str join "."
}

# Splits an argument list on the commas that are not inside brackets.
def split-arguments [text: string]: nothing -> list<string> {
  mut out = []
  mut depth = 0
  mut current = ""
  for char in ($text | split chars) {
    # A literal struct is one argument however many commas it holds:
    # `{ i32, ptr addrspace(5), i32, i32 }` was four before the brace was
    # counted.
    if $char in ["<" "(" "[" "{"] { $depth = $depth + 1 }
    if $char in [">" ")" "]" "}"] { $depth = $depth - 1 }
    if $char == "," and $depth == 0 {
      $out = ($out | append ($current | str trim))
      $current = ""
    } else {
      $current = $current + $char
    }
  }
  if ($current | str trim) != "" { $out = ($out | append ($current | str trim)) }
  $out
}

# One position's type, with the operand name and the attributes taken off.
def bare-type [text: string]: nothing -> string {
  $text
  | str replace --regex '\s+(<[^<>]*>|%\S+)\s*$' ''
  | str replace --all --regex '\b(immarg|readonly|writeonly|readnone|nocapture|noalias|inreg|zeroext|signext|noundef|nonnull|returned|dereferenceable\([0-9]+\)|align [0-9]+|byval\([^)]*\)|range\([^)]*\))\b' ''
  | str replace --all --regex '  +' ' '
  | str trim
}

# The lane count a type carries, or null where it is not a vector. A scalable
# vector counts separately from a fixed one of the same length: `<vscale x 4 x
# i1>` and `<4 x i1>` are not the same shape and upstream does not read one
# where the other is wanted.
def lanes-of [text: string]: nothing -> string {
  let scalable = ($text | parse --regex '^<vscale x (?P<count>[0-9]+) x ')
  if not ($scalable | is-empty) {
    return $"nx($scalable | get count.0)"
  }
  let fixed = ($text | parse --regex '^<(?P<count>[0-9]+) x ')
  if not ($fixed | is-empty) {
    return ($fixed | get count.0)
  }
  ""
}

# The equality classes the tied pairs describe, as a union-find would give
# them: `llvm.fshl` ties every pair among nought, one, two and three, which
# is one class rather than six facts.
def classes [pairs: list, arity: int]: nothing -> list {
  mut groups = []
  for pair in $pairs {
    let left = ($pair | first)
    let right = ($pair | last)
    let touching = ($groups | enumerate | where {|g| ($left in $g.item) or ($right in $g.item)})
    if ($touching | is-empty) {
      $groups = ($groups | append [[$left, $right]])
    } else {
      let merged = ($touching | each {|g| $g.item} | flatten | append [$left, $right] | uniq | sort)
      let keep = ($touching | each {|g| $g.index})
      $groups = ($groups | enumerate | where {|g| $g.index not-in $keep} | each {|g| $g.item})
      $groups = ($groups | append [$merged])
    }
  }
  $groups | each {|g| $g | sort} | sort
}

def main [tree: path, out: path] {
  let langref = ([$tree "llvm" "docs" "LangRef.rst"] | path join)
  if not ($langref | path exists) {
    error make {msg: $"no LangRef at ($langref)"}
  }

  let declares = (
    open --raw $langref
    | lines
    | each {|line| $line | str trim}
    | where ($it | str starts-with "declare ")
    | parse --regex '^declare\s+(?P<result>.+?)\s+@(?P<name>llvm\.[A-Za-z0-9_.]*[A-Za-z0-9_])\s*\((?P<arguments>.*?)\)\s*(#[0-9]+)?$'
  )

  let rows = (
    $declares
    | each {|row|
      {
        name: (strip-mangling $row.name)
        types: ([(bare-type $row.result)] ++ (split-arguments $row.arguments | each {|a| bare-type $a}))
      }
    }
    | where not ($it.types | any {|t| $t | str contains "..."})
    | where not ($it.types | any {|t| $t == ""})
  )

  mut entries = []
  for group in ($rows | group-by name | transpose name rows) {
    let arities = ($group.rows | each {|r| $r.types | length} | uniq)
    if ($arities | length) != 1 { continue }
    let arity = ($arities | first)
    # One instantiation cannot tell "varies together" from "written once".
    if ($group.rows | length) < 2 { continue }
    mut pairs = []
    for left in 0..<$arity {
      let seen = ($group.rows | each {|r| $r.types | get $left} | uniq)
      # A position that never varies is fixed rather than overloaded, and
      # the signature table already states what it is fixed to.
      if ($seen | length) < 2 { continue }
      for right in ($left + 1)..<$arity {
        if ($group.rows | all {|r| ($r.types | get $left) == ($r.types | get $right)}) {
          $pairs = ($pairs | append [[$left, $right]])
        }
      }
    }
    # The same reading again, on lane counts rather than types. A mask is
    # `<4 x i1>` where the value it masks is `<4 x double>`, so the two are
    # not one type and are one shape, and a call giving them different
    # lengths names an instantiation there is not.
    mut lane_pairs = []
    for left in 0..<$arity {
      let seen = ($group.rows | each {|r| lanes-of ($r.types | get $left)} | uniq)
      # A position that is not a vector everywhere has no lane count to tie,
      # and one whose count never varies is fixed rather than tied.
      if ($seen | any {|l| $l == ""}) { continue }
      if ($seen | length) < 2 { continue }
      for right in ($left + 1)..<$arity {
        let other = ($group.rows | each {|r| lanes-of ($r.types | get $right)} | uniq)
        if ($other | any {|l| $l == ""}) { continue }
        if ($group.rows | all {|r| (lanes-of ($r.types | get $left)) == (lanes-of ($r.types | get $right))}) {
          $lane_pairs = ($lane_pairs | append [[$left, $right]])
        }
      }
    }
    if ($pairs | is-empty) and ($lane_pairs | is-empty) { continue }
    $entries = ($entries | append {
      name: $group.name
      classes: (classes $pairs $arity)
      lanes: (classes $lane_pairs $arity)
      arity: $arity
    })
  }

  let sorted = ($entries | sort-by name)
  let write_classes = {|entry, which|
    let written = (
      $entry | get $which
      | each {|class| $"&[" + ($class | each {|p| $"($p)"} | str join ", ") + "]"}
      | str join ", "
    )
    $"    \(\"($entry.name)\", ($entry.arity), &[($written)]\),"
  }
  let body = (
    $sorted
    | where {|entry| ($entry.classes | is-not-empty)}
    | each {|entry| do $write_classes $entry "classes"}
    | str join "\n"
  )
  let lane_body = (
    $sorted
    | where {|entry| ($entry.lanes | is-not-empty)}
    | each {|entry| do $write_classes $entry "lanes"}
    | str join "\n"
  )

  let header = "//! Which positions of an intrinsic have to be the same type.
//!
//! Generated by `corpus/intrinsic-overloads.nu`, which explains the
//! derivation. In short: LangRef documents an overloaded intrinsic once per
//! instantiation, so two positions whose types vary *together* across all of
//! them are one overloaded type, and a call giving them different types is
//! calling something that does not exist. A position whose type never varies
//! is fixed rather than tied, and `table::signature` is what states those.
//!
//! Position 0 is the result and the arguments follow it, upstream tying a
//! result to an argument as readily as two arguments to each other.

/// The positions of one intrinsic, in classes that each have to agree.
/// `ARITY` counts the result as well, so it is one more than the number of
/// arguments.
type Entry = (&'static str, usize, &'static [&'static [usize]]);

/// The classes for the intrinsic this name instantiates, with the arity they
/// were measured at, or `None` when LangRef documents it once or never
/// varies it.
///
/// The reduction is `super::candidates`, which tries the whole name first
/// and then drops trailing mangled types, stopping at the first component
/// that is a word. Dropping any trailing component instead walks past a name
/// into a shorter one that is merely a prefix of it:
/// `llvm.vp.cttz.elts.i32.nxv16i1` would reach `llvm.vp.cttz`, whose result
/// is its operand's type where `vp.cttz.elts` counts into an `i32`, and
/// tying those refuses a module upstream reads.
pub fn tied(name: &str) -> Option<(usize, &'static [&'static [usize]])> {
    super::candidates(name).find_map(|candidate| {
        let index = TIED
            .binary_search_by_key(&candidate, |(name, _, _)| *name)
            .ok()?;
        Some((TIED[index].1, TIED[index].2))
    })
}

/// The classes tied by lane count rather than by type, read the same way.
///
/// A mask is `<4 x i1>` where the value it masks is `<4 x double>`, so the
/// two are not one type and are one shape: `llvm.masked.load` is documented
/// at sixteen lanes, at two and at eight, and its mask is as wide as its
/// result in each. A call giving them different lengths names an
/// instantiation there is not. A position that is not a vector in every
/// documented instantiation has no lane count to tie, and one whose count
/// never varies is fixed rather than tied.
pub fn tied_lanes(name: &str) -> Option<(usize, &'static [&'static [usize]])> {
    super::candidates(name).find_map(|candidate| {
        let index = TIED_LANES
            .binary_search_by_key(&candidate, |(name, _, _)| *name)
            .ok()?;
        Some((TIED_LANES[index].1, TIED_LANES[index].2))
    })
}

/// Sorted, so the lookup can be a binary search.
static TIED: &[Entry] = &["

  [
    $header
    $body
    "];"
    ""
    "/// Sorted, so the lookup can be a binary search."
    "static TIED_LANES: &[Entry] = &["
    $lane_body
    "];"
    ""
  ] | str join "\n" | save --force $out
  print $"($sorted | where {|e| $e.classes | is-not-empty} | length) with tied types, ($sorted | where {|e| $e.lanes | is-not-empty} | length) with tied lanes, into ($out)"
}
