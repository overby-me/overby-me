#!/usr/bin/env nu
# Generates the table of intrinsics upstream renames as it reads them.
#
#   nu intrinsic-renames.nu <llvm-source-tree> <llvm-as> <llvm-dis> <out.rs>
#
# Some intrinsics are read under one name and written under another.
# `llvm.wasm.laneselect` comes back `llvm.wasm.relaxed.laneselect`, and the
# call sites come with it. This is not the mangling suffix, which
# `corpus/intrinsic-mangling.nu` measures: the name itself is different, an
# older spelling upgraded to the current one.
#
# The oracle is the same as everywhere here, a declaration written out and
# read back, and the only difficulty is matching an answer to its question.
# A batch of declarations comes back as a batch of names, and a renamed one
# cannot be matched by name, that being the whole point. So the batch is
# compared as sets: a name that went in and did not come out was renamed to
# a name that came out and did not go in. Where exactly one of each is left
# the pairing is forced; where more are left the batch is halved and asked
# again, until each answer is alone with its question.

def strip-attributes [line: string]: nothing -> string {
  let at = ($line | str index-of "@")
  if $at < 0 { return $line }
  mut depth = 0
  mut index = 0
  mut cut = ($line | str length)
  for char in ($line | split chars) {
    if $index > $at {
      if $char == "(" { $depth = $depth + 1 }
      if $char == ")" {
        $depth = $depth - 1
        if $depth == 0 { $cut = $index + 1; break }
      }
    }
    $index = $index + 1
  }
  $line
  | str substring ..<$cut
  | str replace --all --regex '\b(immarg|readonly|writeonly|readnone|nocapture|noalias|inreg|zeroext|signext|noext|noundef|nonnull|returned|writable|allocptr|allocalign|dead_on_unwind|swiftself|swifterror|nofree|nest|inalloca)\b' ''
  | str replace --all --regex '\b(captures|byval|byref|sret|elementtype|preallocated|nofpclass|range|initializes|dereferenceable|dereferenceable_or_null|alignstack)\([^)]*\)' ''
  | str replace --all --regex '\balign\s+[0-9]+' ''
  | str replace --all --regex '  +' ' '
  | str replace --all --regex ' ,' ','
  | str replace --all --regex ' \)' ')'
  | str trim
}

# Whether a component of a name is a spelled type rather than part of the
# name, the same grammar `crates/llvm-ir/src/intrinsic/reduce.rs` applies.
def spelled [part: string]: nothing -> bool {
  if $part in [
    "Metadata" "bf16" "bfloat" "double" "f128" "f16" "f32" "f64" "f80" "float"
    "fp128" "half" "isVoid" "label" "metadata" "ppcf128" "ptr" "token" "void" "x86amx"
  ] { return true }
  if ($part | str starts-with "sl_") { return ($part | str ends-with "s") }
  if ($part =~ '^[ip][0-9]+$') { return true }
  let composite = ($part | parse --regex '^(?P<prefix>nxv|v|a)(?P<count>[0-9]+)(?P<element>.+)$')
  if ($composite | is-not-empty) {
    return (spelled ($composite | first | get element))
  }
  false
}

# The name with its instantiation types dropped, which is what tells a rename
# from a remangling. Both look the same from outside: one name goes in and
# another comes out. Only a rename changes the name itself, so only a pair
# whose *bases* differ is one.
def base-of [name: string]: nothing -> string {
  mut parts = ($name | split row ".")
  while (($parts | length) > 2 and (spelled ($parts | last))) {
    $parts = ($parts | drop 1)
  }
  $parts | str join "."
}

def declared-name [line: string]: nothing -> string {
  let found = ($line | parse --regex '@(?P<name>llvm\.[A-Za-z0-9_.]*[A-Za-z0-9_])')
  if ($found | is-empty) { "" } else { $found | first | get name }
}

# The names one batch comes back as, or nothing when it will not assemble.
def read-back [lines: list<string>, llvm_as: path, llvm_dis: path, work: path]: nothing -> list<string> {
  let source = ([$work "batch.ll"] | path join)
  let bitcode = ([$work "batch.bc"] | path join)
  $lines | str join "\n" | save --force $source
  let assembled = (do { ^$llvm_as --disable-verify $source -o $bitcode } | complete)
  if $assembled.exit_code != 0 { return [] }
  let printed = (do { ^$llvm_dis $bitcode -o - } | complete)
  if $printed.exit_code != 0 { return [] }
  $printed.stdout
  | lines
  | where ($it | str starts-with "declare ")
  | each {|line| declared-name $line}
  | where ($it | is-not-empty)
}

# The renames one batch shows, halving it while more than one is in doubt.
def renames-in [lines: list<string>, llvm_as: path, llvm_dis: path, work: path]: nothing -> list {
  if ($lines | is-empty) { return [] }
  let asked = ($lines | each {|line| declared-name $line} | where ($it | is-not-empty))
  let answered = (read-back $lines $llvm_as $llvm_dis $work)
  if ($answered | is-empty) {
    # Unassemblable as a batch. One line on its own says whether it is the
    # line or the company it keeps.
    if ($lines | length) == 1 { return [] }
    let half = (($lines | length) // 2)
    let left = (renames-in ($lines | first $half) $llvm_as $llvm_dis $work)
    let right = (renames-in ($lines | skip $half) $llvm_as $llvm_dis $work)
    return ($left ++ $right)
  }
  let gone = ($asked | where {|name| $name not-in $answered})
  let new = ($answered | where {|name| $name not-in $asked})
  if ($gone | is-empty) { return [] }
  if ($gone | length) == 1 and ($new | length) == 1 {
    return [{from: ($gone | first), to: ($new | first)}]
  }
  if ($lines | length) == 1 {
    # One line, and its name did not come back. Whatever came out that did
    # not go in is what it became; nothing came out means it was dropped.
    if ($new | length) == 1 {
      return [{from: ($gone | first), to: ($new | first)}]
    }
    return [{from: ($gone | first), to: ""}]
  }
  let half = (($lines | length) // 2)
  let left = (renames-in ($lines | first $half) $llvm_as $llvm_dis $work)
  let right = (renames-in ($lines | skip $half) $llvm_as $llvm_dis $work)
  $left ++ $right
}

def main [tree: path, llvm_as: path, llvm_dis: path, out: path] {
  let root = ([$tree "llvm" "test"] | path join)
  let lines = (
    ^grep -rhE '^declare .*@llvm\.' --include='*.ll' $root
    | lines
    | uniq
    | each {|line| strip-attributes $line}
    | where ($it | str ends-with ")")
    | uniq
  )
  print $"($lines | length) declare lines harvested"

  # One declaration per name in a batch, a module holding no name twice.
  let rounds = (
    $lines
    | each {|line| {name: (declared-name $line), line: $line}}
    | where name != ""
    | group-by name
    | transpose name rows
    | each {|group| $group.rows | enumerate | each {|item| {round: $item.index, line: $item.item.line}}}
    | flatten
    | group-by round
    | transpose round rows
    | each {|group| $group.rows | get line}
  )
  print $"($rounds | length) rounds"

  let work = (mktemp -d)
  mut found = []
  for batch in $rounds {
    $found = ($found ++ (renames-in $batch $llvm_as $llvm_dis $work))
  }
  rm -rf $work

  # Only a pair whose bases differ is a rename. The rest are remanglings,
  # which look identical from here (a name goes in, another comes out) and
  # are `super::mangling`'s business: recording them here said
  # `llvm.smax.v4i32` was renamed, and said it two different ways for two
  # signatures, which is what 2,719 conflicts were.
  let pairs = (
    $found
    | where to != ""
    | each {|row| {from: (base-of $row.from), to: (base-of $row.to)}}
    | where {|row| $row.from != $row.to}
    | uniq
  )
  let conflicting = (
    $pairs | group-by from | transpose from rows | where {|g| ($g.rows | length) > 1} | get from
  )
  for name in $conflicting { print $"conflicting: ($name)" }
  let rows = ($pairs | where {|row| $row.from not-in $conflicting} | sort-by from)
  let dropped = ($found | where to == "" | get from | uniq | length)
  print $"($rows | length) renames, ($conflicting | length) conflicting, ($dropped) names dropped rather than renamed"

  let body = ($rows | each {|row| $"    \(\"($row.from)\", \"($row.to)\"\),"} | str join "\n")
  let header = "//! The intrinsics upstream renames as it reads them.
//!
//! Generated by `corpus/intrinsic-renames.nu`, which explains the
//! derivation. Some intrinsics are read under one name and written under
//! another: `llvm.wasm.laneselect` comes back
//! `llvm.wasm.relaxed.laneselect`, an older spelling upgraded to the current
//! one, and the call sites come with it.
//!
//! This is not the mangling suffix, which `super::mangling` measures. The
//! name itself is different, so the two are asked separately and the rename
//! is applied first: what a renamed intrinsic is called decides what its
//! components hang off.

/// The name upstream reads this one as, if it reads it as another.
pub fn renamed(name: &str) -> Option<&'static str> {
    let index = RENAMES
        .binary_search_by_key(&name, |(from, _)| *from)
        .ok()?;
    Some(RENAMES[index].1)
}

/// Sorted, so the lookup can be a binary search.
static RENAMES: &[(&str, &str)] = &["

  [$header, $body, "];", ""] | str join "\n" | save --force $out
  ^rustfmt --edition 2021 $out
  print $"written to ($out)"
}
