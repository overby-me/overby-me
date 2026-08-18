#!/usr/bin/env nu
# Generates the order upstream prints attributes in.
#
#   nu attribute-order.nu <opt> <attribute.rs> <out.rs>
#
# An attribute set is not a list: upstream holds it sorted and prints it in
# its own order, whatever order the module wrote. `noalias nocapture
# sret(%T)` comes back `noalias sret(%T) captures(none)`, so a printer that
# keeps what was written prints a different module from upstream's.
#
# The order is measured pairwise. Two attributes are written on one
# declaration and the answer is which of them comes back first, which is one
# edge; the edges sort into a sequence. Asking pairwise rather than putting
# them all on one declaration is what lets a pair that cannot sit together be
# an absent edge rather than a refused module: `byval` and `sret` are two
# ways of passing the same argument and upstream refuses both at once, and
# `range` wants an integer where `nofpclass` wants a float.
#
# Each pair is asked in every position it fits, first one wins: a parameter,
# a return, or the function itself. A pair no position reads is a pair with
# no answer, and there are plenty of those: `byval` and `sret` never sit
# together, and neither do `memory` and `align`, one being a function's and
# the other a parameter's, so an order between them is not a thing to get
# wrong.
#
# The attributes with no argument are in it too, because they are not
# alphabetical: upstream prints `noalias noundef nonnull readonly`, which was
# the assumption this replaces. Their keywords come from the parser's own
# list, so an attribute the parser learns is an attribute this places.
# What stays outside the table is the quoted ones, `"target-cpu"="x86-64"`
# and the like, which print last and sort among themselves by key.

# The spellings, one per attribute that takes an argument. A spelling is the
# question's setup rather than its answer: it says how to write the
# attribute, not where it prints.
const SPELLINGS = [
  [keyword, written];
  ["align", "align 8"]
  ["alignstack", "alignstack(8)"]
  ["allockind", "allockind(\"alloc\")"]
  ["allocsize", "allocsize(0)"]
  ["byref", "byref(i32)"]
  ["byval", "byval(i32)"]
  ["captures", "captures(none)"]
  ["dereferenceable", "dereferenceable(8)"]
  ["dereferenceable_or_null", "dereferenceable_or_null(8)"]
  ["elementtype", "elementtype(i32)"]
  ["inalloca", "inalloca(i32)"]
  ["initializes", "initializes((0, 4))"]
  ["memory", "memory(none)"]
  ["nofpclass", "nofpclass(nan)"]
  ["preallocated", "preallocated(i32)"]
  ["range", "range(i32 0, 8)"]
  ["sret", "sret(i32)"]
  ["uwtable", "uwtable(sync)"]
  ["vscale_range", "vscale_range(1,4)"]
]

# The positions, in the order they are tried. A pair that fits more than one
# is asked in the first, because the answer is the same in all of them and
# asking twice only costs time: the sequences from every position merge into
# one, and a disagreement would show up as a cycle.
const POSITIONS = [
  "param-ptr"
  "param-int"
  "param-float"
  "param-returned"
  "intrinsic"
  "return"
  "function"
  "function-unnamed"
]

def declaration [position: string, index: int, first: string, second: string]: nothing -> string {
  let both = $"($first) ($second)"
  match $position {
    "param-ptr" => $"declare void @p($index)\(ptr ($both))"
    "param-int" => $"declare void @p($index)\(i32 ($both))"
    "param-float" => $"declare void @p($index)\(float ($both))"
    # `returned` says the result is this argument, so it needs a result of
    # the argument's type to be that.
    "param-returned" => $"declare ptr @p($index)\(ptr ($both))"
    # `immarg` is refused anywhere but an intrinsic, and refused there beside
    # anything but `range`, which upstream says in as many words: "Attribute
    # 'immarg' is incompatible with other attributes except the 'range'
    # attribute".
    "intrinsic" => $"declare void @llvm.p($index)\(i32 ($both))"
    "return" => $"declare ($both) i32 @p($index)\()"
    # A function with a parameter and a pointer result, because `allocsize`
    # names a parameter and has nothing to name without one.
    "function" => $"declare ptr @p($index)\(i32) ($both)"
    # And `jumptable` needs the address of the function not to matter.
    "function-unnamed" => $"declare ptr @p($index)\(i32) unnamed_addr ($both)"
    _ => ""
  }
}

# The attribute text upstream printed for one probe, wherever that position
# keeps it.
def printed-attributes [
  position: string,
  index: int,
  lines: list<string>,
  groups: record,
]: nothing -> string {
  let line = ($lines | where {|line| $line =~ $"@\(llvm\\.\)?p($index)\\\(" } | first 1 | str join "")
  if $line == "" { return "" }
  match $position {
    "param-ptr" | "param-int" | "param-float" | "param-returned" | "intrinsic" => {
      let open = ($line | str index-of "(")
      let close = ($line | str index-of --end ")")
      if $open < 0 or $close <= $open { "" } else { $line | str substring ($open + 1)..<$close }
    }
    "return" => ($line | parse --regex '^declare (?P<middle>.*?) @p' | get middle.0? | default "")
    "function" | "function-unnamed" => {
      let number = ($line | parse --regex '#(?P<number>[0-9]+)\s*$' | get number.0? | default "")
      if $number == "" { "" } else { $groups | get --optional $number | default "" }
    }
    _ => ""
  }
}

# Where a keyword sits in a printed attribute list, or -1. The keyword is
# looked for whole: `align` is not `alignstack`, and `dereferenceable` is not
# `dereferenceable_or_null`, which a plain search would get wrong both ways.
def position-of [text: string, keyword: string]: nothing -> int {
  const MARK = "\u{0001}"
  let marked = ($text | str replace --regex $"\\b($keyword)\\b" $MARK)
  if $marked == $text { return (-1) }
  $marked | str index-of $MARK
}

# Which of the two comes first in what upstream printed, or nothing when the
# text does not hold both.
def edge [text: string, first: string, second: string]: nothing -> string {
  let at_first = (position-of $text $first)
  let at_second = (position-of $text $second)
  if $at_first < 0 or $at_second < 0 { return "" }
  if $at_first < $at_second { $"($first)>($second)" } else { $"($second)>($first)" }
}

# One batch of probes, halved down to the line that caused a refusal so that
# one bad pair does not take the rest of the batch with it.
# Each pair is written both ways round, and an answer counts only when the
# two agree. That is what says the order is upstream's own rather than the
# one the module happened to be written in, and it is the whole question
# here: a set held as written would come back both ways.
def ask [opt: path, work: path, position: string, pairs: list]: nothing -> list {
  mut queue = [$pairs]
  mut out = []
  while not ($queue | is-empty) {
    let current = ($queue | first)
    $queue = ($queue | skip 1)
    let text = (
      $current
      | enumerate
      | each {|it|
        [
          (declaration $position ($it.index * 2) $it.item.first $it.item.second)
          (declaration $position ($it.index * 2 + 1) $it.item.second $it.item.first)
        ]
        | str join "\n"
      }
      | str join "\n"
    )
    let source = ([$work "probe.ll"] | path join)
    ($text + "\n") | save --force $source
    let run = (try { do { ^$opt -S -passes=verify $source -o - } | complete } catch { {exit_code: 1} })
    if $run.exit_code != 0 {
      if ($current | length) > 1 {
        let half = (($current | length) // 2)
        $queue = ([($current | first $half), ($current | skip $half)] ++ $queue)
      }
      continue
    }
    let lines = ($run.stdout | lines)
    let groups = (
      $lines
      | each {|line| $line | parse --regex '^attributes #(?P<number>[0-9]+) = \{ (?P<text>.*) \}$'}
      | flatten
      | reduce --fold {} {|row, acc| $acc | upsert $row.number $row.text}
    )
    for it in ($current | enumerate) {
      let written = (printed-attributes $position ($it.index * 2) $lines $groups)
      let reversed = (printed-attributes $position ($it.index * 2 + 1) $lines $groups)
      let one = (edge $written $it.item.left $it.item.right)
      let other = (edge $reversed $it.item.left $it.item.right)
      if $one == "" or $other == "" { continue }
      if $one != $other {
        print $"($it.item.left) and ($it.item.right) come back in the order they were written"
        continue
      }
      $out = ($out | append {pair: $"($it.item.left)|($it.item.right)", edge: $one})
    }
  }
  $out
}

# Kahn's, with the name as the tie-break so that a run with the same edges
# gives the same table.
def sequence [names: list<string>, edges: list<string>]: nothing -> list<string> {
  let parsed = ($edges | each {|edge| $edge | split row ">"} | each {|parts| {before: $parts.0, after: $parts.1}})
  mut remaining = ($names | sort)
  mut out = []
  while not ($remaining | is-empty) {
    let blocked = (
      $parsed
      | where {|edge| ($edge.before in $remaining) and ($edge.after in $remaining)}
      | get after
    )
    let ready = ($remaining | where {|name| $name not-in $blocked})
    if ($ready | is-empty) {
      print $"a cycle among ($remaining | str join ' ')"
      $out = ($out ++ $remaining)
      break
    }
    let next = ($ready | first)
    $out = ($out | append $next)
    $remaining = ($remaining | where {|name| $name != $next})
  }
  $out
}

def main [opt: path, source: path, out: path] {
  let work = (mktemp -d)
  # The attributes written as a bare keyword, taken from the parser's list so
  # that the two cannot drift apart.
  let bare = (
    open $source
    | lines
    | each {|line| $line | parse --regex '^\s+[A-Za-z0-9]+ => "(?P<keyword>[a-z_0-9]+)",$'}
    | flatten
    | get keyword
    | where {|keyword| $keyword not-in ($SPELLINGS | get keyword)}
    | uniq
    | each {|keyword| {keyword: $keyword, written: $keyword}}
  )
  let spellings = ($SPELLINGS ++ $bare)
  print $"($bare | length) attributes with no argument, ($spellings | length) in all"
  let keywords = ($spellings | get keyword)
  mut pairs = []
  for left in ($keywords | enumerate) {
    for right in ($keywords | enumerate) {
      if $right.index <= $left.index { continue }
      $pairs = ($pairs | append {left: $left.item, right: $right.item})
    }
  }
  print $"($pairs | length) pairs to place"

  mut answered = {}
  for position in $POSITIONS {
    let asking = (
      $pairs
      | where {|pair| $"($pair.left)|($pair.right)" not-in ($answered | columns)}
      | each {|pair|
        {
          first: ($spellings | where keyword == $pair.left | get written.0),
          second: ($spellings | where keyword == $pair.right | get written.0),
          left: $pair.left,
          right: $pair.right,
        }
      }
    )
    if ($asking | is-empty) { continue }
    let found = (ask $opt $work $position $asking)
    for row in $found {
      $answered = ($answered | upsert $row.pair $row.edge)
    }
    print $"($position): ($found | length) placed, ($answered | columns | length) of ($pairs | length) answered"
  }
  rm -rf $work

  let edges = ($answered | values)
  let order = (sequence $keywords $edges)
  print $"order: ($order | str join ' ')"
  let unplaced = (
    $keywords
    | where {|name| not ($edges | any {|edge| $edge | str contains $name})}
  )
  if ($unplaced | is-not-empty) {
    print $"nothing placed: ($unplaced | str join ' ')"
  }

  let body = (
    $order
    | enumerate
    | sort-by item
    | each {|it| $"    \(\"($it.item)\", ($it.index)),"}
    | str join "\n"
  )
  let header = "//! The order upstream prints attributes in.
//!
//! Generated by `corpus/attribute-order.nu`, which explains the derivation.
//! In short: an attribute set is held sorted rather than written out as it
//! was read, so `noalias nocapture sret(%T)` comes back
//! `noalias sret(%T) captures(none)`. Two attributes on one declaration say
//! which of them comes first, asked both ways round so that the answer is
//! upstream's order rather than the one the probe happened to write.
//!
//! The quoted attributes are not here. They print after every keyword and
//! sort among themselves by key.
//!
//! Two keywords no declaration can place keep whatever place the sort gave
//! them: upstream allows `elementtype` only on a call site, and `nocapture`
//! never comes back at all, being read as `captures(none)`.

/// Where this attribute sorts, or `usize::MAX` for a keyword nothing
/// measured placed.
pub fn rank(keyword: &str) -> usize {
    ORDER
        .binary_search_by_key(&keyword, |(known, _)| *known)
        .map_or(usize::MAX, |index| ORDER[index].1)
}

/// Sorted by keyword so the lookup is a binary search; the number beside
/// each is its place in the measured order.
static ORDER: &[(&str, usize)] = &["

  [$header, $body, "];", ""] | str join "\n" | save --force $out
  ^rustfmt --edition 2021 $out
  print $"written to ($out)"
}
