#!/usr/bin/env nu
# Generates the set of intrinsic names upstream builds a declaration for.
#
#   nu intrinsic-recognised.nu <llvm-source-tree> <llvm-as> <out.rs>
#
# An intrinsic needs no declaration: upstream recognises the name at the call
# and materialises one from the call's own signature. A name it does not
# recognise is "use of undefined value" instead, so the two are not the same
# thing and only the first may be built. LangRef names 421 intrinsics, which
# is where `corpus/intrinsic-names.nu` stops, and upstream recognises many
# more: the coroutine, exception-handling and debug-info intrinsics are
# documented in other files entirely, `llvm.vector.interleave4` is documented
# nowhere, and every target's intrinsics are documented only in the target
# backend. Four hundred and eighty-three modules across the eleven tree
# ratchets were refused for that one reason.
#
# The derivation needs no probing, because upstream has already answered.
# Take a file `llvm-as` reads, and a name it uses that the file itself never
# gives a body to: upstream must have recognised that name and built the
# declaration, since nothing else would let the module resolve. The exit code
# of `llvm-as` on the file is the whole oracle, and the names fall out of a
# scan of the files it accepts.
#
# Two things are taken out of the scan, both of which made it answer a
# question other than the one asked. Comments go first: half of these files
# are FileCheck lines quoting IR, and a name upstream never parsed says
# nothing about what it recognises. And a name the file defines as a global
# is not an intrinsic at all, `@llvm.used` and `@llvm.global_ctors` being
# reserved globals; counting those would have us build a function declaration
# for a variable.
#
# What is stored is the name with its instantiation types dropped, which is
# what a lookup reduces to anyway, and collapses the thousands of spellings
# the cost-model tests carry into the names behind them.

# Whether a component of a name is a spelled type rather than part of the
# name, which is the grammar `corpus/intrinsic-mangling.nu` measured read
# backwards. The same predicate `crates/llvm-ir/src/intrinsic/reduce.rs`
# applies, and it has to stay the same one: this decides what is stored and
# that decides what is found.
#
# "Has a digit in it" is the loose version and is wrong in both directions.
# `interleave4` is not a type, and reducing through it stored `llvm.vector`;
# `llvm.amdgcn.fdot2` stored `llvm.amdgcn`, which would answer for every name
# that target has.
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

# The name with its trailing spelled types dropped, stopping at the first
# component that is a word.
def strip-mangling [name: string]: nothing -> string {
  mut parts = ($name | split row ".")
  while (($parts | length) > 2 and (spelled ($parts | last))) {
    $parts = ($parts | drop 1)
  }
  $parts | str join "."
}

def main [tree: path, llvm_as: path, out: path] {
  let root = ([$tree "llvm" "test"] | path join)
  if not ($root | path exists) {
    error make {msg: $"no test tree at ($root)"}
  }
  # Every directory upstream's tests have, not only the eleven the tree
  # ratchets cover. A name is recognised or it is not, and the suite a module
  # happens to live in says nothing about that: `llvm.amdgcn.ds.append` is
  # named only in `Assembler`, and leaving that directory out left one of the
  # three modules we refuse refused for want of a name upstream knows.
  let trees = (
    ls $root
    | where type == dir
    | get name
    | each {|path| $path | path basename}
    | sort
  )

  # A record used as a set. A list would be the obvious thing and is the
  # wrong one: `append` copies, so collecting names one at a time into a
  # growing list is quadratic.
  mut names = {}
  mut read = 0
  for suite in $trees {
    let files = (glob $"($root)/($suite)/**/*.ll" | sort)
    for file in $files {
      # The scan comes before the assembler, not after. Nearly every file
      # names only intrinsics some earlier file already offered, and running
      # `llvm-as` on all thirty-seven thousand of them to learn nothing new
      # is most of the cost of this script. A file that offers nothing new
      # cannot change the answer, so it is never assembled.
      let text = (open --raw $file | str replace --all --regex '(?m);.*$' '')
      # Both halves take the symbol exactly as written, to the end of what a
      # name may hold, rather than trimming a trailing separator off it. A
      # symbol that ends in a dot is a different symbol, and upstream's own
      # tests contain one: `wmma-gfx12-w32.ll` declares
      # `@llvm.amdgcn.wmma.i32.16x16x16.iu8.v8i32.v2i32.` and calls the name
      # without the dot, so the call is undeclared and upstream materialises
      # it. A scan that trimmed the dot read the typo as the declaration and
      # lost the name.
      let defined = (
        $text
        | parse --regex '(?m)^(?:declare |define |@)(?P<rest>[^\n]*)'
        | get rest
        | each {|line| $line | parse --regex '@?(?P<name>llvm\.[A-Za-z0-9_.]*)' | get name}
        | flatten
      )
      let offered = (
        $text
        | parse --regex '@(?P<name>llvm\.[A-Za-z0-9_.]*)'
        | get name
        | uniq
        | where {|name| $name not-in $defined}
        | each {|name| strip-mangling $name}
        | uniq
        | where {|base| $base not-in $names}
      )
      if ($offered | is-empty) { continue }
      # Only now is the oracle worth asking: does upstream read this module?
      let upstream = (do { ^$llvm_as $file -o /dev/null } | complete)
      if $upstream.exit_code != 0 { continue }
      $read = $read + 1
      for base in $offered {
        $names = ($names | upsert $base ($file | path basename))
      }
    }
    print $"($suite): ($names | columns | length) names so far"
  }
  print $"($read) files were the first to offer a name, and upstream reads each"

  let sorted = ($names | columns | sort)
  let body = ($sorted | each {|name| $"    \"($name)\","} | str join "\n")
  let header = "//! The intrinsic names upstream builds a declaration for.
//!
//! Generated by `corpus/intrinsic-recognised.nu`, which explains the
//! derivation. In short: an intrinsic needs no declaration, upstream
//! recognising the name at the call and materialising one from the call's own
//! signature, and a name it does not recognise is \"use of undefined value\"
//! instead. LangRef names 421 of them and upstream knows many more, the
//! coroutine and exception-handling ones being documented in other files and
//! every target's in the target backend, so this is measured from the modules
//! upstream reads rather than from any document.
//!
//! A name here is one that a file `llvm-as` reads uses without giving it a
//! body, which upstream could only resolve by recognising it. The name is
//! stored with its instantiation types dropped, which is what a lookup
//! reduces to.

/// Whether upstream would build a declaration for this name.
pub fn is_recognised(name: &str) -> bool {
    super::candidates(name).any(names)
}

/// Whether this exact name is in the table, which is what the reduction asks
/// while it is working out what to reduce to.
pub fn names(name: &str) -> bool {
    RECOGNISED.binary_search(&name).is_ok()
}

/// Sorted, so the lookup can be a binary search.
static RECOGNISED: &[&str] = &["

  [$header, $body, "];", ""] | str join "\n" | save --force $out
  ^rustfmt --edition 2021 $out
  print $"($sorted | length) names into ($out)"
}
