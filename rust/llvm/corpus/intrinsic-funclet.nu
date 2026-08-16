#!/usr/bin/env nu
# Generates which intrinsics need a funclet token when called inside one.
#
#   nu intrinsic-funclet.nu <llvm-source-tree> <opt> <out.rs>
#
# Windows exception handling puts a call inside a funclet, and a call that may
# be lowered into a real function call has to say which funclet it is in:
# `call ptr @llvm.objc.retain(ptr null) [ "funclet"(token %pad) ]`. Upstream
# refuses one that does not, "Missing funclet token on intrinsic call", and
# `Verifier/operand-bundles-wineh.ll` is that module.
#
# It is a property of the intrinsic rather than of the call. An ordinary call
# needs no bundle there, and neither do `llvm.memcpy`, `llvm.stacksave`,
# `llvm.eh.typeid.for` or `llvm.launder.invariant.group`, while every
# `llvm.objc.*` probed does. Which set that is is what this measures, rather
# than guessing at the family from six probes.
#
# Each name is asked twice: once with a funclet bundle on the call and once
# without. A name refused both ways is refused for a reason that has nothing
# to do with the bundle, which is most of them, `llvm.va_start` wanting a
# variadic caller and an `immarg` parameter wanting a literal where this
# writes `undef`. Only a name refused without the bundle and read with it
# needs one, and asking both ways is the whole of what tells them apart.

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

# A declaration with the attributes taken off, leaving the types alone.
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

# One probe: a declaration, and a function that calls it from inside a
# funclet the Windows personality opens.
def probe-text [index: int, row: record, bundle: bool]: nothing -> string {
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
  let token = if $bundle { ' [ "funclet"(token %pad) ]' } else { "" }
  let call = if $row.ret == "void" {
    $"  call void @($row.name)\(($args))($token)"
  } else {
    $"  %r = call ($row.ret) @($row.name)\(($args))($token)"
  }
  [
    $"declare ($row.ret) @($row.name)\(($row.params | str join ', '))"
    $"define void @p($index)\(($taken)) personality ptr @__CxxFrameHandler3 {"
    "entry:"
    $"  invoke void @may_throw\() to label %cont unwind label %disp"
    "disp:"
    "  %cs = catchswitch within none [label %catch] unwind to caller"
    "catch:"
    "  %pad = catchpad within %cs [ptr null, i32 0, ptr null]"
    "  br label %body"
    "body:"
    $call
    "  catchret from %pad to label %cont"
    "cont:"
    "  ret void"
    "}"
  ] | str join "\n"
}

def main [tree: path, opt: path, out: path] {
  let work = (mktemp -d)

  let written = (
    ^grep -rh -E '^declare .*@llvm\.' $"($tree)/llvm/test"
    | lines
    | each {|line| $line | str trim}
    | where {|line| not ($line =~ '\.\.\.')}
    | uniq
  )
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
      # defined in the probe, so those are left out rather than guessed at.
      if ($params | any {|p| $p == "" or ($p =~ '%')}) { return null }
      if ($row.ret =~ '%') { return null }
      {name: $row.name, ret: ($row.ret | str trim), params: $params}
    }
    | compact
    | uniq-by name
  )
  print $"($rows | length) intrinsics to ask about"

  # Which names a batch reads, as a set. A refused batch is halved and each
  # half asked again, down to the line that caused it, so one bad probe does
  # not take two hundred answers with it.
  def read-batch [opt: path, work: path, slice: list, bundle: bool]: nothing -> list {
    mut queue = [$slice]
    mut read = []
    while not ($queue | is-empty) {
      let current = ($queue | first)
      $queue = ($queue | skip 1)
      let text = (
        ($current | enumerate | each {|it| probe-text $it.index $it.item $bundle} | str join "\n\n")
        + "\n\ndeclare void @may_throw()\ndeclare i32 @__CxxFrameHandler3(...)\n!0 = !{}\n"
      )
      let source = ([$work "probe.ll"] | path join)
      $text | save --force $source
      let run = (
        try { do { ^$opt -S -passes=verify $source -o /dev/null } | complete }
        catch { {exit_code: 1} }
      )
      if $run.exit_code == 0 {
        $read = ($read | append ($current | get name))
        continue
      }
      if ($current | length) > 1 {
        let half = (($current | length) // 2)
        $queue = ([($current | first $half), ($current | skip $half)] ++ $queue)
      }
    }
    $read
  }

  mut without = []
  mut with = []
  let batch = 100
  mut start = 0
  while $start < ($rows | length) {
    let slice = ($rows | skip $start | first $batch)
    $without = ($without ++ (read-batch $opt $work $slice false))
    $with = ($with ++ (read-batch $opt $work $slice true))
    $start = $start + $batch
  }
  rm -rf $work

  # Read with a token and refused without one: the bundle is what it wanted.
  let needs = ($with | where {|name| $name not-in $without} | uniq | sort)
  print $"($without | length) read with no bundle, ($with | length) read with one"
  print $"($needs | length) need a funclet token: ($needs | str join ' ')"

  let body = ($needs | each {|name| $"    \"($name)\","} | str join "\n")
  let header = "//! The intrinsics that need a funclet token inside a funclet.
//!
//! Generated by `corpus/intrinsic-funclet.nu`, which explains the derivation.
//! In short: Windows exception handling puts a call inside a funclet, and a
//! call upstream may lower into a real function call has to say which funclet
//! it is in. Upstream refuses one that does not, and which intrinsics those
//! are is measured by asking each one twice, with a bundle and without.
//!
//! An ordinary call needs no bundle there and neither do most intrinsics, so
//! a name with no row here is one that may be called plainly.

/// Whether a call to this name inside a funclet needs a `funclet` bundle.
pub fn needs_funclet_token(name: &str) -> bool {
    let base = super::base_name(name);
    NEEDS.iter().any(|known| super::base_name(known) == base)
}

/// Sorted, so a reader can find a name in it.
static NEEDS: &[&str] = &["

  [$header, $body, "];", ""] | str join "\n" | save --force $out
  ^rustfmt --edition 2021 $out
  print $"written to ($out)"
}
