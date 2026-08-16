#!/usr/bin/env nu
# Generates the table of which positions of an intrinsic go into its name.
#
#   nu intrinsic-mangling.nu <llvm-source-tree> <llvm-as> <llvm-dis> <out.rs>
#
# An overloaded intrinsic carries the types it was instantiated at in its own
# name: `llvm.umax` at `i8` is `llvm.umax.i8`, and `llvm.memcpy` between two
# flat pointers with a 64-bit length is `llvm.memcpy.p0.p0.i64`. A module may
# write the name without them, and upstream fills them in, which is why
# `declare void @llvm.lifetime.start(i64, ptr)` comes back as
# `@llvm.lifetime.start.p0`. Printing the name as written is a difference from
# upstream on every such module, so we have to be able to fill them in too.
#
# Which positions those are is not a rule that can be read off a signature.
# `llvm.masked.load` is `<2 x double> (ptr, i32, <2 x i1>, <2 x double>)` and
# its name is `llvm.masked.load.v2f64.p0`: the result and the first argument
# go in, the mask does not, though the mask varies with the result as surely
# as the passthrough does. So it is measured, one intrinsic at a time.
#
# The measurement has two halves, and neither is reasoned about:
#
#   * What a type spells. `llvm.ssa.copy` is overloaded on a single position
#     that takes any first class type, so `declare T @llvm.ssa.copy(T)` comes
#     back named `@llvm.ssa.copy.<spelling of T>`. One module, one answer.
#     `crates/llvm-ir/src/intrinsic/mangle.rs` is the same function written in
#     Rust, and its unit tests are these answers.
#
#   * What the whole name is. Writing a bare `declare` of the base name with
#     a documented signature and reading back the name upstream gave it says
#     which components it wanted, in which order.
#
# The positions then follow by matching the components against the spellings
# of the signature's own types, left to right. Where more than one assignment
# fits, the ones LangRef documents more than once cut it down; where an
# ambiguity survives every instantiation, nothing distinguishes the choices on
# anything LangRef documents, so the first is taken. Where no assignment fits
# at all, the row is dropped rather than guessed: a wrong row renames a
# function upstream leaves alone.

# Splits an argument list on the commas that are not inside brackets.
def split-arguments [text: string]: nothing -> list<string> {
  mut out = []
  mut depth = 0
  mut current = ""
  for char in ($text | split chars) {
    if $char in ["<" "(" "["] { $depth = $depth + 1 }
    if $char in [">" ")" "]"] { $depth = $depth - 1 }
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
#
# The list is longer than the one `corpus/intrinsic-overloads.nu` carries
# because a type that keeps an attribute on it does not spell anything, and a
# position that spells nothing loses the whole intrinsic here rather than one
# pair. `captures(none)` alone accounts for `llvm.lifetime.start` and
# `llvm.invariant.start`, which are two of the ones this table exists for.
def bare-type [text: string]: nothing -> string {
  $text
  | str replace --regex '\s+(<[^<>]*>|%\S+)\s*$' ''
  # The ones that take a parameter first, and without a word boundary after
  # the closing bracket: there is no boundary between `)` and the end of the
  # string, so asking for one drops nothing at all. That is what hid
  # `captures(none)`, and with it `llvm.lifetime.start`.
  | str replace --all --regex '\b(dereferenceable|dereferenceable_or_null|alignstack|byval|byref|sret|inalloca|preallocated|elementtype|captures|range|initializes)\([^)]*\)' ''
  | str replace --all --regex '\balign [0-9]+' ''
  | str replace --all --regex '\b(immarg|readonly|writeonly|readnone|nocapture|noalias|inreg|zeroext|signext|noundef|nonnull|returned|nofree|nest|swiftself|swifterror|writable|dead_on_unwind)\b' ''
  | str replace --all --regex '  +' ' '
  | str trim
}

# The `declare` lines LangRef holds, with the ones it wraps across lines put
# back together. It wraps by argument, so a line is finished when its
# brackets balance; `llvm.memcpy` is written over two lines and is one of the
# intrinsics this table exists for.
def declare-lines [text: string]: nothing -> list<string> {
  mut out = []
  mut pending = ""
  for raw in ($text | lines) {
    let line = ($raw | str trim)
    if $pending == "" {
      if not ($line | str starts-with "declare ") { continue }
      $pending = $line
    } else {
      $pending = $"($pending) ($line)"
    }
    let opened = ($pending | split chars | where {|c| $c == "("} | length)
    let closed = ($pending | split chars | where {|c| $c == ")"} | length)
    if $opened > 0 and $opened == $closed {
      $out = ($out | append $pending)
      $pending = ""
    }
    # A wrapped declare never runs past a blank line in LangRef, and a line
    # that opens no bracket at all is prose rather than a signature.
    if $opened == 0 { $pending = "" }
  }
  $out
}

# Whether a component of a name is a spelled type rather than part of the
# name. Same reading as `crates/llvm-ir/src/intrinsic/mod.rs`: a spelling
# usually carries a width or a count, and the ones that do not are a closed
# set.
def spelled [part: string]: nothing -> bool {
  ($part =~ '[0-9]') or ($part in [
    "bfloat" "double" "float" "half" "isVoid" "metadata" "Metadata" "ptr" "token" "void"
  ])
}

# The names a lookup tries, longest first: the whole name, then the whole
# name with its trailing spelled types dropped one at a time. The same
# reduction `super::candidates` performs in Rust, so that what this measures
# is what that finds.
def candidates [name: string]: nothing -> list<string> {
  mut out = [$name]
  mut parts = ($name | split row ".")
  while (($parts | length) > 2 and (spelled ($parts | last))) {
    $parts = ($parts | drop 1)
    $out = ($out | append ($parts | str join "."))
  }
  $out
}

# The name with its trailing spelled types dropped, stopping at the first
# component that is a word.
def strip-mangling [name: string]: nothing -> string {
  mut parts = ($name | split row ".")
  while (($parts | length) > 2 and (spelled ($parts | last))) {
    $parts = ($parts | drop 1)
  }
  $parts | str join "."
}

# Runs one module through the assembler and gives back the `llvm.*` names it
# printed, or nothing when it refused the module.
def assemble [as: path, dis: path, work: path, text: string]: nothing -> list<string> {
  let source = ([$work "probe.ll"] | path join)
  let bitcode = ([$work "probe.bc"] | path join)
  $text | save --force $source
  let assembled = (do { ^$as -disable-verify $source -o $bitcode } | complete)
  if $assembled.exit_code != 0 { return [] }
  let printed = (do { ^$dis $bitcode -o - } | complete)
  if $printed.exit_code != 0 { return [] }
  $printed.stdout
  | lines
  | where ($it | str starts-with "declare ")
  | each {|line| $line | parse --regex '@(?P<name>llvm\.[^(]*)\(' | get name}
  | flatten
}

# The names the assembler gave a batch of bare declares, halving the batch
# when it refuses one. LangRef writes some of its signatures in prose rather
# than in IR, and a module is refused whole, so one unusable line would
# otherwise take every line beside it with it.
def probe-batch [as: path, dis: path, work: path, rows: list]: nothing -> list<string> {
  if ($rows | is-empty) { return [] }
  let text = (
    $rows
    | each {|row| $"declare ($row.result) @($row.base)\(($row.arguments | str join ', ')\)"}
    | str join "\n"
  )
  let printed = (assemble $as $dis $work $text)
  if ($printed | is-not-empty) { return $printed }
  if ($rows | length) == 1 { return [] }
  let half = (($rows | length) // 2)
  let left = (probe-batch $as $dis $work ($rows | first $half))
  let right = (probe-batch $as $dis $work ($rows | skip $half))
  $left ++ $right
}

# What one type spells, asked of the assembler through the intrinsic that is
# overloaded on any first class type at all.
#
# The type goes inside a target extension type rather than standing on its
# own, because `void` is not a value and `declare void @llvm.ssa.copy(void)`
# is refused, while `target("w", void)` is read and spells `tw_isVoidt`. A
# target type spells its own name and then each parameter behind an
# underscore, so the spelling wanted is what is left after `tw_` and before
# the closing `t`. Wrapping every type the same way keeps one path rather
# than two, and `void` is the result of nearly a third of the intrinsics
# here.
def spell [as: path, dis: path, work: path, type: string]: nothing -> string {
  let wrapped = $"target\(\"w\", ($type)\)"
  let names = (assemble $as $dis $work $"declare ($wrapped) @llvm.ssa.copy\(($wrapped)\)")
  if ($names | length) != 1 { return "" }
  let name = ($names | first)
  if not ($name | str starts-with "llvm.ssa.copy.tw_") { return "" }
  let spelled = ($name | str substring 17..)
  if not ($spelled | str ends-with "t") { return "" }
  $spelled | str substring ..<(($spelled | str length) - 1)
}

# The same type at a different width or address space, which is what tells
# two positions apart when they happen to spell the same thing in the one
# signature LangRef documents. `llvm.invariant.start` returns a `ptr` and
# takes a `ptr`, and only asking it about a `ptr addrspace(1)` says which of
# the two its name is built from. A type with no neighbour answers itself, and
# the caller learns nothing from it.
def mutate [type: string]: nothing -> string {
  let pointer = ($type | parse --regex '^ptr addrspace\((?P<space>[0-9]+)\)$')
  if ($pointer | is-not-empty) {
    let space = ($pointer | first | get space | into int)
    return $"ptr addrspace\(($space + 1)\)"
  }
  if $type == "ptr" { return "ptr addrspace(1)" }
  let integer = ($type | parse --regex '^i(?P<bits>[0-9]+)$')
  if ($integer | is-not-empty) {
    let bits = ($integer | first | get bits | into int)
    return $"i($bits * 2)"
  }
  let vector = ($type | parse --regex '^<(?P<count>[0-9]+) x (?P<element>.+)>$')
  if ($vector | is-not-empty) {
    let row = ($vector | first)
    return $"<(($row.count | into int) * 2) x ($row.element)>"
  }
  match $type {
    "half" => "float",
    "bfloat" => "float",
    "float" => "double",
    "double" => "float",
    "x86_fp80" => "fp128",
    "fp128" => "x86_fp80",
    _ => $type,
  }
}

# Every assignment of the components to positions, left to right. A component
# is taken by a position whose type spells it, and positions only move
# forwards, which is the order a name is built in.
def assignments [components: list<string>, spellings: list<string>]: nothing -> list<list<int>> {
  mut partial = [[]]
  for component in $components {
    mut next = []
    for taken in $partial {
      let after = (if ($taken | is-empty) { 0 } else { ($taken | last) + 1 })
      for position in $after..<($spellings | length) {
        if ($spellings | get $position) == $component {
          $next = ($next | append [($taken | append $position)])
        }
      }
    }
    # A signature that repeats one type in many positions can fan out without
    # bound, and nothing that wide is decided by taking more of it.
    if ($next | length) > 2000 { return [] }
    $partial = $next
  }
  $partial
}

def main [tree: path, as: path, dis: path, out: path] {
  let langref = ([$tree "llvm" "docs" "LangRef.rst"] | path join)
  if not ($langref | path exists) {
    error make {msg: $"no LangRef at ($langref)"}
  }
  let work = (mktemp -d)

  # Upstream's own tests declare thousands of intrinsics LangRef never
  # mentions, every target's among them, and a name with no row here prints
  # as the module wrote it where upstream fills in its types. The same lines
  # are what the rows are held against further down, so they are read once.
  let harvest = (
    ^grep -rhoE '^declare[^{]*@llvm\.[A-Za-z0-9_.]*\([^)]*\)' --include='*.ll' ([$tree "llvm" "test"] | path join)
    | lines
    | uniq
    | parse --regex '^declare\s+(?P<result>.+?)\s+@(?P<name>llvm\.[A-Za-z0-9_.]*[A-Za-z0-9_])\s*\((?P<arguments>.*?)\)$'
    | each {|row|
      {
        name: $row.name
        types: ([(bare-type $row.result)] ++ (split-arguments $row.arguments | each {|a| bare-type $a}))
      }
    }
    | where not ($it.types | any {|t| ($t == "") or ($t | str contains "...")})
  )
  print $"($harvest | length) intrinsic declarations in llvm/test"

  let declares = (
    declare-lines (open --raw $langref)
    | parse --regex '^declare\s+(?P<result>.+?)\s+@(?P<name>llvm\.[A-Za-z0-9_.]*[A-Za-z0-9_])\s*\((?P<arguments>.*?)\)\s*(#[0-9]+)?$'
    | each {|row|
      {
        base: (strip-mangling $row.name)
        types: ([(bare-type $row.result)] ++ (split-arguments $row.arguments | each {|a| bare-type $a}))
        arguments: (split-arguments $row.arguments | each {|a| bare-type $a})
        result: (bare-type $row.result)
      }
    }
    | where not ($it.types | any {|t| ($t == "") or ($t | str contains "...")})
    | uniq
  )
  print $"($declares | length) usable declare lines in LangRef"

  # A test's declaration is a signature like any other once its name is
  # reduced to the base: writing that base with those types and reading back
  # what upstream calls it is the same question LangRef's lines are asked.
  let declares = (
    $declares
    ++ (
      $harvest
      | each {|row|
        {
          base: (strip-mangling $row.name)
          types: $row.types
          arguments: ($row.types | skip 1)
          result: ($row.types | first)
        }
      }
    )
    | uniq
  )
  print $"($declares | length) signatures once the tests are in"

  # One module per instantiation depth, so that a base declared twice never
  # collides with itself. Names inside a module are distinct, and an output
  # name is matched to the longest base that prefixes it, which is
  # unambiguous because the first thing after a base is a spelled type and a
  # longer base's next component is a word.
  let grouped = ($declares | group-by base | transpose base rows)
  let depth = ($grouped | each {|g| $g.rows | length} | math max)
  mut canonical = {}
  for round in 0..<$depth {
    let batch = ($grouped | each {|g| $g.rows | get -o $round} | compact)
    let printed = (probe-batch $as $dis $work $batch)
    for name in $printed {
      let fitting = (
        $batch
        | get base
        | where {|b| ($name == $b) or ($name | str starts-with $"($b).")}
        | sort-by {|b| $b | str length}
      )
      if ($fitting | is-empty) { continue }
      let base = ($fitting | last)
      # A base declared in two rounds answers once per round; keep them apart
      # by the round, since the signatures differ.
      $canonical = ($canonical | upsert $"($round)|($base)" $name)
    }
    print $"round ($round): ($batch | length) declares, ($printed | length) names back"
  }

  # What every type in play spells, memoised: the same handful of types runs
  # through hundreds of signatures.
  mut spellings = {}
  for row in $declares {
    for type in $row.types {
      if ($type in $spellings) { continue }
      $spellings = ($spellings | upsert $type (spell $as $dis $work $type))
    }
  }
  let spelling = $spellings
  print $"($spelling | columns | length) distinct types spelled"

  # One entry per base and arity, which is what a row is keyed on. A base
  # declared at two lengths is two intrinsics as far as the name goes, and
  # accumulating one assignment across both gives positions that index past
  # the shorter one. The rounds stay per base, because the probe writes the
  # bare name and two arities of one base cannot share a module.
  mut entries = []
  for pair in ($grouped | each {|g| $g.rows | each {|r| $r.types | length} | uniq | each {|a| {group: $g, arity: $a}}} | flatten) {
    let group = $pair.group
    mut tally = {}
    mut started = false
    for round in 0..<($group.rows | length) {
      let row = ($group.rows | get $round)
      if ($row.types | length) != $pair.arity { continue }
      # A round that came back with nothing says nothing: the batch it was in
      # may have been refused for another line in it, and a base read from
      # upstream's tests has as many rounds as the tests have signatures for
      # it, where one from LangRef has one or two. Discarding the base for a
      # single silent round threw away every target intrinsic declared more
      # than a few times, `llvm.amdgcn.image.atomic.swap.1d` among them.
      let name = ($canonical | get -o $"($round)|($group.base)")
      if $name == null { continue }
      let components = (
        if $name == $group.base { [] } else { $name | str substring (($group.base | str length) + 1).. | split row "." }
      )
      if ($components | is-empty) {
        # Upstream left this one alone, which is what it does with a
        # signature that is not the intrinsic's. Another round may still be.
        continue
      }
      let spelled_types = ($row.types | each {|t| $spelling | get $t})
      if ($spelled_types | any {|s| $s == ""}) { continue }
      let candidates = (assignments $components $spelled_types)
      if ($candidates | is-empty) { continue }
      # Counted rather than intersected. With LangRef alone a base had one or
      # two signatures and every one of them was right, so the assignments
      # they agreed on were the answer. Upstream's tests bring dozens per
      # base and some of them are inventions their own module never compiles,
      # so one odd signature would empty the intersection and take the base
      # with it, `llvm.memcpy` among them. The assignment the most signatures
      # support is the answer instead, and the mutation probes below still
      # cut down whatever ties.
      for candidate in $candidates {
        let key = ($candidate | each {|p| $"($p)"} | str join ",")
        $tally = ($tally | upsert $key (($tally | get -o $key | default 0) + 1))
      }
      $started = true
    }
    if (not $started) {
      continue
    }
    let best = ($tally | values | math max)
    mut fits = (
      $tally
      | transpose key count
      | where count == $best
      | get key
      | each {|k| $k | split row "," | each {|p| $p | into int}}
    )
    if ($fits | is-empty) {
      continue
    }

    # Where the documented instantiations leave more than one assignment
    # standing, ask about a signature LangRef does not document: the same one
    # with a single position moved to a neighbouring width or address space.
    # `llvm.invariant.start` takes a `ptr` and returns one, and only its
    # answer at `ptr addrspace(1)` says which of the two its name carries.
    # A mutation the intrinsic has no instantiation for is not remangled at
    # all, which says nothing and is skipped.
    let first_row = ($group.rows | where {|r| ($r.types | length) == $pair.arity} | first)
    if ($fits | length) > 1 {
      for position in 0..<($first_row.types | length) {
        if ($fits | length) <= 1 { break }
        let original = ($first_row.types | get $position)
        let moved = (mutate $original)
        if $moved == $original { continue }
        let types = ($first_row.types | update $position $moved)
        let probed = (probe-batch $as $dis $work [{
          base: $group.base
          result: ($types | first)
          arguments: ($types | skip 1)
        }])
        if ($probed | length) != 1 { continue }
        let name = ($probed | first)
        if not ($name | str starts-with $"($group.base).") { continue }
        let components = ($name | str substring (($group.base | str length) + 1).. | split row ".")
        let spelled_types = ($types | each {|t| spell $as $dis $work $t})
        if ($spelled_types | any {|s| $s == ""}) { continue }
        let candidates = (assignments $components $spelled_types)
        if ($candidates | is-empty) { continue }
        let narrowed = ($fits | where {|f| $f in $candidates})
        if ($narrowed | is-not-empty) { $fits = $narrowed }
      }
    }

    let chosen = ($fits | sort | first)
    $entries = ($entries | append {
      name: $group.base
      arity: ($first_row.types | length)
      positions: $chosen
      # How many assignments the measurements never told apart, so that the
      # ones taken on faith can be counted rather than assumed away.
      ambiguous: ($fits | length)
    })
  }

  # Every row now says what a name should be. Upstream's own tests are full
  # of intrinsic declarations, so the rows can be held against them: apply a
  # row to a declaration whose base it answers for, and the name it builds
  # should be the one the test wrote. Where it is not, the test may have
  # written an older spelling, which is the whole reason for this table, so
  # the disagreement is put to the assembler rather than counted. A row the
  # assembler contradicts is dropped.

  let by_name = ($entries | reduce --fold {} {|entry, all| $all | upsert $entry.name $entry})
  mut wider = $spelling
  mut disputed = []
  for row in $harvest {
    let base = (candidates $row.name | where {|c| $c in $by_name} | first 1)
    if ($base | is-empty) { continue }
    let entry = ($by_name | get ($base | first))
    if $entry.arity != ($row.types | length) { continue }
    mut spelled = []
    mut usable = true
    for type in $row.types {
      if not ($type in $wider) {
        $wider = ($wider | upsert $type (spell $as $dis $work $type))
      }
      let text = ($wider | get $type)
      if $text == "" { $usable = false; break }
      $spelled = ($spelled | append $text)
    }
    if not $usable { continue }
    let components = $spelled
    let built = ([$entry.name] ++ ($entry.positions | each {|p| $components | get $p}) | str join ".")
    if $built != $row.name {
      $disputed = ($disputed | append {name: $entry.name, written: $row.name, built: $built, types: $row.types})
    }
  }
  print $"($disputed | length) declarations a row does not build the written name for"

  # One probe each: what upstream calls the very declaration the test wrote.
  # Agreeing with us clears the row, and the test was carrying an older name.
  mut refuted = []
  mut confirmed = 0
  mut silent = 0
  for dispute in ($disputed | uniq) {
    if ($dispute.name in $refuted) { continue }
    let probed = (probe-batch $as $dis $work [{
      base: $dispute.written
      result: ($dispute.types | first)
      arguments: ($dispute.types | skip 1)
    }])
    if ($probed | length) != 1 { $silent = $silent + 1; continue }
    let upstream = ($probed | first)
    if $upstream == $dispute.built {
      $confirmed = $confirmed + 1
    } else {
      print $"  dropped ($dispute.name): ($dispute.written) is ($upstream) upstream, not ($dispute.built)"
      $refuted = ($refuted | append $dispute.name)
    }
  }
  print $"($confirmed) of them are names upstream rewrites the same way we would"
  print $"($silent) the assembler would not read at all"
  print $"($refuted | length) rows dropped for disagreeing with the assembler"

  let sorted = ($entries | where {|e| $e.name not-in $refuted} | sort-by name)
  let body = (
    $sorted
    | each {|entry|
      let written = ($entry.positions | each {|p| $"($p)"} | str join ", ")
      $"    \(\"($entry.name)\", ($entry.arity), &[($written)]\),"
    }
    | str join "\n"
  )

  let header = "//! Which positions of an intrinsic go into its name.
//!
//! Generated by `corpus/intrinsic-mangling.nu`, which explains the
//! derivation. In short: an overloaded intrinsic carries the types it was
//! instantiated at in its own name, a module may write the name without them,
//! and upstream fills them in. Writing a bare `declare` of a documented
//! signature and reading back the name upstream gave it says which components
//! it wanted; matching those against the spellings of the signature's own
//! types says which positions they came from.
//!
//! Position 0 is the result and the arguments follow it, because a result
//! goes into a name as readily as an argument: `llvm.masked.load` is
//! `llvm.masked.load.v2f64.p0`, whose first component is what it returns.
//!
//! Only the intrinsics LangRef documents are here. A name with no row keeps
//! whatever the module wrote, which is what we did before this table existed.

/// The positions one intrinsic's name is built from, at the arity they were
/// measured at. `ARITY` counts the result, so it is one more than the number
/// of arguments.
type Entry = (&'static str, usize, &'static [usize]);

/// The name without its components, the arity they were measured at, and the
/// positions whose types go into it. `None` when the intrinsic is not
/// overloaded or LangRef does not document it.
///
/// The reduction is `super::candidates`, the same one the other tables use:
/// the whole name first, then trailing spelled types dropped one at a time.
/// That is what lets a name that already carries some of its components be
/// recognised, which matters because the ones written for typed pointers
/// carry all but the ones the pointers used to imply.
pub fn positions(name: &str) -> Option<(&'static str, usize, &'static [usize])> {
    super::candidates(name).find_map(|candidate| {
        let index = MANGLED
            .binary_search_by_key(&candidate, |(name, _, _)| *name)
            .ok()?;
        Some(MANGLED[index])
    })
}

/// Sorted, so the lookup can be a binary search.
static MANGLED: &[Entry] = &["

  [$header, $body, "];", ""] | str join "\n" | save --force $out
  # Written the way `cargo fmt` would write it, so that regenerating the
  # table is a no-op rather than a diff the formatter then undoes.
  ^rustfmt --edition 2021 $out
  rm -rf $work
  let undecided = ($sorted | where ambiguous > 1 | length)
  print $"($sorted | length) intrinsics with a name built from their types into ($out)"
  print $"($undecided) of them had positions nothing measured told apart"
}
