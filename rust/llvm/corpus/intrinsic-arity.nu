#!/usr/bin/env nu
# What upstream makes of an intrinsic written at an older arity.
#
#   nu intrinsic-arity.nu <llvm-source-tree> <opt> <out.rs>
#
# An intrinsic that has gained a parameter since a module was written is read
# through an upgrade rather than refused. `declare i8 @llvm.ctlz.i8(i8)` comes
# back as `declare i8 @llvm.ctlz.i8(i8, i1 immarg)`, and every call to it comes
# back with an `i1 false` appended; `llvm.objectsize.i32(ptr, i1)` gains two of
# them and a mangling suffix with it. A few intrinsics go the other way and are
# dropped outright, call and declaration together.
#
# So the derivation is: write the declaration out, call it with what it says it
# takes, and read back what upstream made of the call. The written arity is the
# question and the printed one is the answer, which needs no guess about which
# arities ever existed: upstream's own tests write the old spellings, that being
# what they are testing.
#
# The probe is read back through `opt -S` with the verifier off, because the
# upgrade is the reader's work and the verifier would refuse the arguments the
# probe writes: an `immarg` parameter takes a literal and this writes `undef`
# everywhere, having no way to know what a literal would mean.
#
# Each intrinsic gets a function of its own, `@p<index>`, because an upgrade may
# rename as well as re-arity and a batch of calls could not otherwise be matched
# back to what was written.
#
# The call is given that function's own parameters rather than constants, and
# that is what tells an argument upstream synthesises from one it worked out.
# Written with constants, `llvm.x86.avx512.mask.load.d` comes back with the
# passthrough it folded out of them, which reads exactly like a constant
# upstream appended; written with parameters it comes back mentioning one, and
# the row is dropped. It also makes an expansion visible: an intrinsic upstream
# rewrites into other instructions leaves them behind, where the same call on
# constants folds away to nothing and reads like a drop.
#
# The name is recorded as written. Reducing it here would need the mangling
# grammar `crates/llvm-ir/src/intrinsic/reduce.rs` measured, and a loose
# reduction is worse than none: "a component with a digit" turned
# `llvm.aarch64.sve.ld2.sret.nxv16i8` into `llvm.aarch64.sve`, which as a key
# answers for every intrinsic that target has. The lookup reduces both sides
# with the measured one instead.

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

# A declaration with the attributes taken off, leaving the types alone. The
# same cleaning `corpus/intrinsic-attributes.nu` does to a written line, and
# for the same reason: what is being asked about is the shape rather than what
# the module happened to write on it.
def clean-written [line: string]: nothing -> string {
  $line
  | str replace --all --regex '\b(immarg|readonly|writeonly|readnone|nocapture|noalias|inreg|zeroext|signext|noext|noundef|nonnull|returned|writable|allocptr|allocalign|dead_on_unwind|swiftself|swifterror|nofree|nest|inalloca|nounwind|willreturn|speculatable|nocallback|nosync|cold|noreturn|convergent|norecurse|mustprogress)\b' ''
  | str replace --all --regex '\b(captures|byval|byref|sret|elementtype|preallocated|nofpclass|range|initializes|dereferenceable|dereferenceable_or_null|alignstack|memory)\([^)]*\)' ''
  | str replace --all --regex '\balign\s+[0-9]+' ''
  | str replace --all --regex ' #[0-9]+' ''
  | str replace --all --regex '  +' ' '
  | str replace --all --regex ' ,' ','
  | str replace --all --regex ' \)' ')'
  | str trim
}

# Whether a parameter of this type can be one of the probe function's own.
# Two cannot: metadata is not a value a function takes, and a token is one
# only an intrinsic may be given.
def is_passable [ty: string]: nothing -> bool {
  ($ty | str trim) not-in ["token" "metadata"]
}

# The value a parameter of this type is called with: the probe function's own
# parameter wherever it can be, so that anything upstream works out of it
# comes back mentioning it.
def argument-for [ty: string, index: int]: nothing -> string {
  let ty = ($ty | str trim)
  if $ty == "token" {
    "token none"
  } else if $ty == "metadata" {
    "metadata !0"
  } else {
    $"($ty) %a($index)"
  }
}

# One probe: a declaration and a function that calls it with its own
# parameters.
def probe-text [index: int, row: record]: nothing -> string {
  let args = (
    $row.params | enumerate | each {|it| argument-for $it.item $it.index} | str join ', '
  )
  let taken = (
    $row.params
    | enumerate
    | where {|it| is_passable $it.item}
    | each {|it| $"($it.item) %a($it.index)"}
    | str join ', '
  )
  let call = if $row.ret == "void" {
    $"  call void @($row.name)\(($args))"
  } else {
    $"  %r = call ($row.ret) @($row.name)\(($args))"
  }
  [
    $"declare ($row.ret) @($row.name)\(($row.params | str join ', '))"
    $"define void @p($index)\(($taken)) {"
    $call
    "  ret void"
    "}"
  ] | str join "\n"
}

# What one batch's output says: the call each probe function came back with.
def read-probes [text: string]: nothing -> record {
  mut out = {}
  mut current = ""
  for line in ($text | lines) {
    let opened = ($line | parse --regex '^define void @p(?P<index>[0-9]+)\(')
    if not ($opened | is-empty) {
      $current = ($opened | first | get index)
      $out = ($out | upsert $current {name: "", args: [], rest: false})
      continue
    }
    if $current == "" { continue }
    let called = (
      $line
      | parse --regex '^\s*(?:%[A-Za-z0-9_.]+ = )?(?:tail )?call [^@]*@(?P<name>[A-Za-z0-9_.$]+)\((?P<args>.*)\)$'
    )
    if not ($called | is-empty) {
      let row = ($called | first)
      let held = ($out | get $current)
      $out = (
        $out | upsert $current {name: $row.name, args: (split-arguments $row.args), rest: $held.rest}
      )
      continue
    }
    let text = ($line | str trim)
    if $text == "}" {
      $current = ""
      continue
    }
    # Anything else with an opcode in it is what an expansion left behind.
    if $text != "" and $text != "ret void" {
      let held = ($out | get $current)
      $out = ($out | upsert $current {name: $held.name, args: $held.args, rest: true})
    }
  }
  $out
}

def main [tree: path, opt: path, out: path] {
  let work = (mktemp -d)

  # Every `declare` of an `llvm.` name upstream's own tests write, one per
  # name. The tests are where the old spellings live, this being what they
  # test; nothing else enumerates the arities that used to exist.
  #
  # One `grep` over the whole tree rather than one per file: thirty-seven
  # thousand processes is most of an hour where a single recursive search is
  # seconds, and the dedupe is one pass at the end for the same reason.
  let written = (
    ^grep -rh -E '^declare .*@llvm\.' $"($tree)/llvm/test"
    | lines
    | each {|line| $line | str trim}
    | where {|line| not ($line =~ '\.\.\.')}
    | uniq
  )
  print $"($written | length) distinct declarations written"
  let rows = (
    $written
    | each {|line|
      let parsed = (
        clean-written $line
        | parse --regex '^declare (?P<ret>.*?)\s*@(?P<name>llvm\.[A-Za-z0-9_.]*[A-Za-z0-9_])\((?P<args>.*)\)$'
      )
      if ($parsed | is-empty) { return null }
      let row = ($parsed | first)
      let params = (
        if ($row.args | str trim) == "" { [] } else { split-arguments $row.args }
      )
      # A declaration naming a struct type by name would need that type
      # defined in the probe, and defining it would change what the name
      # mangles to. Those are left out rather than guessed at.
      if ($params | any {|p| $p == "" or ($p =~ '%')}) { return null }
      if ($row.ret =~ '%') { return null }
      {
        name: $row.name
        ret: ($row.ret | str trim)
        params: $params
        shape: $"($row.name)|($params | str join ',')"
      }
    }
    | compact
    | uniq-by shape
  )
  print $"($rows | length) distinct intrinsic declarations to ask about"

  # A batch upstream will not read says nothing about any line in it, so a
  # refused one is halved and each half asked again, down to the single line
  # that caused it. Those are counted rather than hidden: a declaration this
  # cannot put to the assembler is one the sweep did not measure.
  mut upgrades = []
  mut dropped = []
  mut expanded = []
  mut derived = []
  mut renamed = []
  mut unusable = 0
  mut queue = []
  let batch = 200
  mut start = 0
  while $start < ($rows | length) {
    $queue = ($queue | append [($rows | skip $start | first $batch)])
    $start = $start + $batch
  }
  while not ($queue | is-empty) {
    let slice = ($queue | first)
    $queue = ($queue | skip 1)
    let text = (
      ($slice | enumerate | each {|it| probe-text $it.index $it.item} | str join "\n\n")
      + "\n\n!0 = !{}\n"
    )
    let source = ([$work "probe.ll"] | path join)
    $text | save --force $source
    # A probe upstream crashes on is no verdict either, and it takes the
    # whole run with it unless the crash is caught here.
    let run = (
      try { do { ^$opt -S -disable-verify $source -o - } | complete }
      catch { {exit_code: 1, stdout: ""} }
    )
    if $run.exit_code != 0 {
      if ($slice | length) == 1 {
        $unusable = $unusable + 1
      } else {
        let half = (($slice | length) // 2)
        $queue = ([($slice | first $half), ($slice | skip $half)] ++ $queue)
      }
      continue
    }
    let read = (read-probes $run.stdout)
    for it in ($slice | enumerate) {
      let answer = ($read | get --optional $"($it.index)")
      if $answer == null { continue }
      if $answer.name == "" {
        # Nothing left at all is a drop, and only for an intrinsic that
        # returns nothing: one whose call folded to a constant leaves no
        # instruction either, and dropping that would leave whatever read
        # the result with nothing to read.
        if $answer.rest or $it.item.ret != "void" {
          $expanded = ($expanded | append $it.item.name)
        } else {
          $dropped = ($dropped | append $it.item.name)
        }
        continue
      }
      # A call that came back under a name this one is not a prefix of was
      # renamed as well as re-aritied, and which name it ends up with is
      # `corpus/intrinsic-renames.nu`'s question rather than this one's.
      if not (($answer.name | str starts-with $it.item.name) or ($it.item.name | str starts-with $answer.name)) {
        $renamed = ($renamed | append $it.item.name)
        continue
      }
      let written = ($it.item.params | length)
      if ($answer.args | length) <= $written { continue }
      # The arguments written have to have survived, in order and unchanged.
      # An expansion leaves a call of its own behind, `llvm.nvvm.rotate.b64`
      # coming back as an `llvm.fshl.i64` whose operands it worked out, and
      # that reads as the same call with an argument appended unless the ones
      # written are checked.
      let kept = (
        $answer.args
        | first $written
        | enumerate
        | all {|arg| $arg.item == (argument-for ($it.item.params | get $arg.index) $arg.index)}
      )
      if not $kept {
        $expanded = ($expanded | append $it.item.name)
        continue
      }
      let added = ($answer.args | skip $written)
      # An argument holding a value is one upstream worked out from the call
      # rather than one it appends, and there is no row for that.
      if ($added | any {|a| $a =~ '%'}) {
        $derived = ($derived | append $it.item.name)
        continue
      }
      $upgrades = ($upgrades | append {
        name: $it.item.name
        arity: $written
        added: $added
      })
    }
  }
  rm -rf $work
  print $"($unusable) declarations the assembler would not read on their own"

  let upgrades = ($upgrades | uniq-by name arity | sort-by name arity)
  let dropped = ($dropped | uniq | sort)
  print $"($upgrades | length) declarations upstream reads at an older arity"
  print $"($dropped | length) it drops outright"
  print $"($expanded | uniq | length) it rewrites into other instructions, which is no table"
  print $"($derived | uniq | length) whose added argument it works out from the call"
  print $"($renamed | uniq | length) it renames as well, which is another table's question"
  for row in $upgrades {
    print $"  ($row.name) at ($row.arity) gains ($row.added | str join ', ')"
  }
  if not ($dropped | is-empty) {
    print $"  dropped: ($dropped | str join ' ')"
  }

  let body = (
    $upgrades
    | each {|row|
      let added = ($row.added | each {|a| $'"($a)"'} | str join ', ')
      $"    \(\"($row.name)\", ($row.arity), &[($added)]),"
    }
    | str join "\n"
  )
  let drops = ($dropped | each {|name| $"    \"($name)\","} | str join "\n")
  let header = "//! The intrinsics upstream reads at an older arity than they have.
//!
//! Generated by `corpus/intrinsic-arity.nu`, which explains the derivation.
//! In short: a declaration written before an intrinsic gained a parameter is
//! upgraded rather than refused, so writing one out and reading back the call
//! says what the added arguments are.
//!
//! This is what upstream's own tests write, the way the rename table is: the
//! old spellings live there because that is what those tests test, and no
//! specification enumerates the arities that used to exist.

/// One upgrade: the base name, the arity it was written at, and the arguments
/// upstream appends to a call, written the way it writes them.
type Entry = (&'static str, u8, &'static [&'static str]);

/// What a call to this name at this arity gains, or `None` when nothing
/// measured says it gains anything.
/// Both sides are reduced to the base name, so a width no test happens to
/// declare is upgraded the way the ones they do declare are. Every row of one
/// base agrees about what it gains, which is what makes that sound: the rows
/// are per instantiation because the sweep asks about a declaration as
/// written, not because an instantiation could differ.
pub fn upgrade(name: &str, arity: usize) -> Option<&'static [&'static str]> {
    let arity = u8::try_from(arity).ok()?;
    let base = super::base_name(name);
    UPGRADES
        .iter()
        .find(|(known, at, _)| *at == arity && super::base_name(known) == base)
        .map(|(_, _, added)| *added)
}

/// Whether upstream drops this intrinsic entirely, its calls with it.
pub fn is_dropped(name: &str) -> bool {
    let base = super::base_name(name);
    DROPPED.iter().any(|known| super::base_name(known) == base)
}

/// Sorted by name and then by the arity written.
static UPGRADES: &[Entry] = &["

  [
    $header
    $body
    "];"
    ""
    "/// Sorted, so the lookup can be a binary search."
    "static DROPPED: &[&str] = &["
    $drops
    "];"
    ""
  ] | str join "\n" | save --force $out
  ^rustfmt --edition 2021 $out
  print $"written to ($out)"
}
