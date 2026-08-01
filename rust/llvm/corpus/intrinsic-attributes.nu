#!/usr/bin/env nu
# Generates the per-intrinsic attribute table by asking upstream for it.
#
#   nu intrinsic-attributes.nu <llvm-source-tree> <llvm-as> <llvm-dis> <out.rs>
#
# An intrinsic carries attributes nothing in the text says. `llvm.assume` is
# `nocallback nofree nosync nounwind willreturn memory(inaccessiblemem: write)`
# with an `i1 noundef` parameter, and a module declaring it plainly gets them
# anyway: upstream replaces whatever a declaration was written with by the
# intrinsic's own set, parameter attributes included. So writing out each
# `declare` line and reading it back is what says the attributes are.
#
# This was recorded as unobtainable for four passes, on the grounds that only
# fourteen of LangRef's eight hundred `declare` lines carry an attribute and
# fourteen is too few to harvest from. That is true and it is the wrong
# question: what LangRef has to supply is the signature, not the attributes.
# The oracle supplies the attributes.
#
# The signature has to be right, because a `declare` whose types do not match
# the intrinsic comes back untouched. That is what tells a line probing
# nothing from one whose intrinsic genuinely carries nothing, and it is what
# makes LangRef's schematic lines (`<<n> x i1>`, `<ty2>`) usable rather than
# lost: a placeholder is filled in with each of several concrete types, and
# the instantiations upstream recognises are the ones that were right.

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

# The attributes upstream printed on one argument, with the type removed.
# A type is the first word (or bracketed run); everything after it is an
# attribute, except the `addrspace(N)` that belongs to a pointer type.
def argument-attributes [argument: string]: nothing -> string {
  let text = ($argument | str trim)
  # Peel the type off the front: a bracketed type is balanced, a plain one
  # runs to the first space.
  mut depth = 0
  mut index = 0
  mut split_at = ($text | str length)
  for char in ($text | split chars) {
    if $char in ["<" "[" "{"] { $depth = $depth + 1 }
    if $char in [">" "]" "}"] { $depth = $depth - 1 }
    if $char == " " and $depth == 0 {
      $split_at = $index
      break
    }
    $index = $index + 1
  }
  let rest = ($text | str substring $split_at.. | str trim)
  # `ptr addrspace(5)` is one type rather than a type and an attribute.
  if ($rest | str starts-with "addrspace(") {
    let after = ($rest | str replace --regex '^addrspace\([0-9]+\)' '' | str trim)
    $after
  } else {
    $rest
  }
}

# LangRef writes some declarations with their function attributes after the
# argument list, `declare void @llvm.trap() cold noreturn nounwind`, so a
# line ends where its argument list does. Thirty-seven do, and dropping them
# for not ending in a bracket cost `llvm.trap` and the whole `memcpy` family.
#
# The scan starts at the `@`, because a return type can hold a bracket of its
# own: `ptr addrspace(5)` would otherwise look like the argument list and
# truncate the name away.
def trim-after-arguments [line: string]: nothing -> string {
  let at = ($line | str index-of "@")
  if $at < 0 { return $line }
  mut depth = 0
  mut index = 0
  for char in ($line | split chars) {
    if $index > $at {
      if $char == "(" { $depth = $depth + 1 }
      if $char == ")" {
        $depth = $depth - 1
        if $depth == 0 { return ($line | str substring ..$index) }
      }
    }
    $index = $index + 1
  }
  $line
}

# LangRef writes operand names and attributes into its `declare` lines, and a
# placeholder where an operand's name would go. None of it survives into the
# probe: the names are not syntax upstream reads in a declaration, and the
# attributes are the thing being asked about.
def clean-declare [line: string]: nothing -> string {
  trim-after-arguments $line
  | str replace --regex '\s*#[0-9]+\s*$' ''
  | str replace --all --regex '\s+(<[a-zA-Z][a-zA-Z0-9_ ]*>|%[A-Za-z0-9_.]+)(\s*[,)])' '$2'
  | str replace --all --regex '\b(immarg|readonly|writeonly|readnone|nocapture|noalias|inreg|zeroext|signext|noext|noundef|nonnull|returned|writable|allocptr|allocalign|dead_on_unwind|swiftself|swifterror)\b' ''
  | str replace --all --regex 'captures\([a-z_, ]*\)' ''
  | str replace --all --regex '\b(align\s+[0-9]+|dereferenceable\([0-9]+\)|dereferenceable_or_null\([0-9]+\)|nofpclass\([a-z ]*\)|range\([^)]*\)|initializes\([^)]*\))' ''
  | str replace --all --regex '  +' ' '
  | str replace --all --regex ' ,' ','
  | str replace --all --regex ' \)' ')'
  | str replace --all --regex '\( ' '('
  | str trim
}

# A line still holding an angle-bracketed word is schematic rather than a
# type: LangRef writes `<<n> x i1>`, `<ty>` and `<ty2>` where the shape is
# the point. A real vector writes its length first, so a body beginning with
# a letter is a placeholder, with one exception: a scalable vector begins
# with `vscale`, and it is put aside rather than mistaken for one.
def schematic [line: string]: nothing -> bool {
  ($line | str replace --all "<vscale x " "") =~ '<[a-zA-Z_][a-zA-Z0-9_ ]*>'
}

# A schematic line is instantiated rather than dropped. LangRef writes the
# constrained floating-point family with `<ty>` where a concrete type goes,
# and that is a whole family of intrinsics to lose.
#
# Which concrete type is right cannot be known here, and does not have to be:
# a declaration whose types do not fit comes back untouched, so a wrong guess
# yields nothing and a right one yields the attributes. Several are tried and
# the ones upstream recognises are the ones that were right. The name is left
# as LangRef writes it, mangling suffix and all, because upstream recomputes
# that from the types anyway.
def substitute [text: string, ty: string]: nothing -> string {
  $text
  | str replace --all "<vscale x " "\u{1}vscale x "
  | str replace --all --regex '<<[a-z][a-z0-9_]*> x <[a-z][a-z0-9_ ]*>>' $"<4 x ($ty)>"
  | str replace --all --regex '<[a-z][a-z0-9_ ]*>' $ty
  | str replace --all "\u{1}vscale x " "<vscale x "
}

def instantiate [line: string]: nothing -> list<string> {
  if not (schematic $line) {
    return [$line]
  }
  let at = ($line | str index-of "@")
  if $at < 0 {
    return []
  }
  # The return type and the arguments are substituted apart, because a
  # conversion takes one kind and produces another: writing `i32` in both
  # halves of `fptosi` describes nothing, and one type everywhere is what
  # left that whole family out.
  let head = ($line | str substring ..<$at)
  let tail = ($line | str substring $at..)
  let types = ["double" "float" "i32" "i64"]
  $types
  | each {|result|
    $types | each {|argument|
      $"(substitute $head $result)(substitute $tail $argument)"
    }
  }
  | flatten
  | where not (schematic $it)
  | uniq
}

# Whether a run of text closes every bracket it opens, which is what says a
# `declare` is whole.
def balanced [text: string]: nothing -> bool {
  mut depth = 0
  for char in ($text | split chars) {
    if $char == "(" { $depth = $depth + 1 }
    if $char == ")" { $depth = $depth - 1 }
  }
  $depth == 0
}

# LangRef wraps a long `declare` across lines, so a line is a whole one only
# when its brackets close. A fragment taken on its own is not a declaration
# and poisons the batch it goes into, which is worth more care than dropping
# it: the constrained floating-point intrinsics are all written this way.
def harvest [all: list<string>]: nothing -> list<string> {
  mut out = []
  mut pending = ""
  mut held = 0
  for raw in $all {
    let line = ($raw | str trim)
    if $pending != "" {
      $pending = $"($pending) ($line)"
      $held = $held + 1
      if (complete-declare $pending) {
        $out = ($out | append $pending)
        $pending = ""
        $held = 0
      } else if $held > 8 {
        # Not a wrapped declaration after all, only a line that starts like
        # one inside prose.
        $pending = ""
        $held = 0
      }
      continue
    }
    if ($line | str starts-with "declare ") {
      if (complete-declare $line) {
        $out = ($out | append $line)
      } else {
        $pending = $line
        $held = 0
      }
    }
  }
  $out | where ($it =~ '@llvm\.')
}

# A declaration is whole when it names something and closes every bracket it
# opens. Both halves matter: LangRef wraps a long declaration across lines
# and puts the return type on one of its own, so `declare <type>` is not a
# declaration and neither is the argument list that follows it. Requiring
# only the brackets to close lost the whole constrained floating-point
# family, whose declarations are all written that way.
def complete-declare [text: string]: nothing -> bool {
  ($text =~ '@') and ($text | str contains "(") and (balanced $text)
}

# The name a `declare` line declares, which is what two candidates for the
# same schematic line share and what a module may only hold one of.
def declared-name [line: string]: nothing -> string {
  let found = ($line | parse --regex '@(?P<name>llvm\.[A-Za-z0-9_.]*[A-Za-z0-9_])')
  if ($found | is-empty) { "" } else { $found | first | get name }
}

# What one round's output says: the attributes on each declaration in it.
def read-declares [text: string]: nothing -> list {
  # `attributes #3 = { ... }` says what a group holds.
  let groups = (
    $text | lines
    | parse --regex '^attributes #(?P<number>[0-9]+) = \{ (?P<body>.*) \}$'
    | reduce --fold {} {|row, acc| $acc | insert $row.number $row.body}
  )
  $text | lines
  | parse --regex '^declare (?P<before>[^@]*)@(?P<name>llvm\.[A-Za-z0-9_.]*[A-Za-z0-9_])\((?P<arguments>.*)\)(?: #(?P<group>[0-9]+))?$'
  | each {|row|
    {
      base: (strip-mangling $row.name)
      instantiation: $row.name
      ret: (return-attributes ($row.before | str trim))
      params: (split-arguments $row.arguments | each {|a| argument-attributes $a})
      function: (if ($row.group | is-empty) { "" } else { $groups | get --optional $row.group | default "" })
    }
  }
}

# Assembles the batch, dropping whatever llvm-as objects to until it reads.
# A line that has to be dropped is a line LangRef writes in a shape this
# does not clean up to legal IR, and the count is reported rather than hidden.
def assemble [lines: list<string>, llvm_as: path, work: path] {
  mut current = $lines
  mut dropped = []
  let source = ([$work "probe.ll"] | path join)
  let bitcode = ([$work "probe.bc"] | path join)
  for _attempt in 0..<3000 {
    $current | str join "\n" | save --force $source
    # The verifier is off because what is being asked about is what the
    # reader does, and a verifier complaint carries no line to narrow by:
    # one instantiation whose types do not suit an intrinsic would otherwise
    # take the whole round with it.
    let run = (^$llvm_as --disable-verify $source -o $bitcode | complete)
    if $run.exit_code == 0 {
      return {bitcode: $bitcode, kept: $current, dropped: $dropped}
    }
    let where = (
      $run.stderr | lines | first
      | parse --regex ':(?P<line>[0-9]+):[0-9]+: error'
    )
    if ($where | is-empty) {
      error make {msg: $"llvm-as failed in a way this cannot narrow: ($run.stderr)"}
    }
    let index = (($where | first | get line | into int) - 1)
    $dropped = ($dropped | append ($current | get $index))
    $current = ($current | drop nth $index)
  }
  error make {msg: $"still failing after 3000 drops; last was ($dropped | last)"}
}

def main [tree: path, llvm_as: path, llvm_dis: path, out: path] {
  let langref = ([$tree "llvm" "docs" "LangRef.rst"] | path join)
  if not ($langref | path exists) {
    error make {msg: $"no LangRef at ($langref)"}
  }
  let work = (mktemp --directory --tmpdir "intrinsic-attributes-XXXXXX")

  let harvested = (
    harvest (open --raw $langref | lines)
    | each {|line| clean-declare $line}
    | each {|line| instantiate $line}
    | flatten
    | where ($it | str ends-with ")")
    | uniq
  )
  print $"($harvested | length) declare lines harvested from LangRef"

  # A module holds one declaration per name, and the candidate
  # instantiations of a schematic line all share theirs, LangRef writing no
  # mangling suffix where the type is a placeholder. So the candidates go in
  # rounds, one per name in each, rather than a batch where fifteen of every
  # sixteen would be thrown out as a redefinition.
  let rounds = (
    $harvested
    | each {|line| {name: (declared-name $line), line: $line}}
    | group-by name
    | transpose name rows
    | each {|group| $group.rows | enumerate | each {|item| {round: $item.index, line: $item.item.line}}}
    | flatten
    | group-by round
    | transpose round rows
    | each {|group| $group.rows | get line}
  )
  print $"($rounds | length) rounds, the first holding ($rounds | first | length)"

  mut declares = []
  mut dropped = 0
  mut probed = 0
  for lines in $rounds {
    let assembled = (assemble $lines $llvm_as $work)
    $dropped = $dropped + ($assembled.dropped | length)
    $probed = $probed + ($assembled.kept | length)
    let printed = (^$llvm_dis $assembled.bitcode -o - | complete)
    if $printed.exit_code != 0 {
      error make {msg: $"llvm-dis failed: ($printed.stderr)"}
    }
    $declares = ($declares | append (read-declares $printed.stdout))
  }
  print $"($dropped) dropped as unassemblable, ($probed) probed"
  print $"($declares | length) declarations read back"

  # An intrinsic's attributes are the intrinsic's, so every instantiation of
  # one base name has to agree. Where they do not, the disagreement is
  # reported and the entry is left out rather than one of them being picked.
  mut entries = []
  mut conflicts = []
  for group in ($declares | group-by base | transpose base rows) {
    # A declaration upstream did not recognise comes back exactly as it was
    # written, which says nothing rather than saying there are no
    # attributes. Comparing one of those against an instantiation that was
    # recognised reports a disagreement where there is only a guess that
    # missed, so they are dropped before the answers are compared.
    let answered = (
      $group.rows
      | where {|row| $row.function != "" or $row.ret != "" or ($row.params | any {|p| $p != ""})}
    )
    if ($answered | is-empty) {
      continue
    }
    let functions = ($answered | each {|r| $r.function} | uniq)
    let rets = ($answered | each {|r| $r.ret} | uniq)
    let params = ($answered | each {|r| $r.params | str join "|"} | uniq)
    if ($functions | length) != 1 or ($rets | length) != 1 or ($params | length) != 1 {
      $conflicts = ($conflicts | append {
        name: $group.base
        functions: $functions
        rets: $rets
        params: $params
      })
      continue
    }
    let first = ($answered | first)
    $entries = ($entries | append {
      name: $group.base
      function: $first.function
      ret: $first.ret
      params: $first.params
    })
  }

  for conflict in $conflicts {
    print $"conflicting: ($conflict.name) ret=($conflict.rets) params=($conflict.params)"
  }
  print $"($entries | length) intrinsics with attributes, ($conflicts | length) conflicting"

  let body = (
    $entries
    | sort-by name
    | each {|entry|
      let params = ($entry.params | each {|p| $'"($p)"'} | str join ", ")
      $"    \(\"($entry.name)\", Attributes { function: \"($entry.function)\", ret: \"($entry.ret)\", params: &[($params)] }\),"
    }
    | str join "\n"
  )

  let header = "//! What attributes upstream gives each intrinsic.
//!
//! Generated by `corpus/intrinsic-attributes.nu`, which explains the
//! derivation. In short: upstream replaces whatever attributes an intrinsic
//! declaration was written with by the intrinsic's own, so writing out each
//! `declare` line LangRef documents and reading it back through
//! `llvm-as | llvm-dis` says what they are. LangRef supplies the signature,
//! which has to be right for the name to be recognised at all; it does not
//! supply the attributes and does not have to.
//!
//! The text is what upstream prints, and is read back through the same
//! attribute parser a module's own attributes go through, so the two cannot
//! drift apart.

/// The attributes one intrinsic carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Attributes {
    /// What goes in the function's attribute group, in upstream's order.
    pub function: &'static str,
    /// What goes before the return type, empty when nothing does.
    pub ret: &'static str,
    /// One entry per parameter, in order, empty where that parameter
    /// carries nothing.
    pub params: &'static [&'static str],
}

/// What upstream gives the intrinsic this name instantiates, or `None` when
/// it is not one whose attributes were measured.
///
/// The reduction is `super::candidates`, which tries the whole name first
/// and then drops trailing mangled types, stopping at the first component
/// that is a word. `llvm.vp.cttz.elts` is not `llvm.vp.cttz`.
pub fn attributes(name: &str) -> Option<&'static Attributes> {
    super::candidates(name).find_map(|candidate| {
        let index = ATTRIBUTES
            .binary_search_by_key(&candidate, |(name, _)| *name)
            .ok()?;
        Some(&ATTRIBUTES[index].1)
    })
}

/// Sorted, so the lookup can be a binary search.
static ATTRIBUTES: &[(&str, Attributes)] = &["

  [$header, $body, "];", ""] | str join "\n" | save --force $out
  rm --recursive --force $work
  print $"written to ($out)"
}

# The return attributes out of what a `declare` writes before the `@`, which
# is the attributes and then the return type.
#
# Taken from the front rather than by peeling the type off the end, because
# a type is not reliably one word: `ptr addrspace(5)` is two and reading the
# second as an attribute made four intrinsics look as though their
# instantiations disagreed. An attribute is one of a known set, and the run
# of them ends where the type begins.
def return-attributes [before: string]: nothing -> string {
  mut rest = ($before | str trim)
  mut taken = []
  loop {
    let matched = (
      $rest | parse --regex '^(?P<one>zeroext|signext|noext|inreg|noalias|nonnull|noundef|dereferenceable_or_null\([0-9]+\)|dereferenceable\([0-9]+\)|align [0-9]+|nofpclass\([a-z ]*\)|range\([^)]*\))(?P<after>\s.*|$)'
    )
    if ($matched | is-empty) { break }
    let row = ($matched | first)
    $taken = ($taken | append $row.one)
    $rest = ($row.after | str trim)
  }
  $taken | str join " "
}
