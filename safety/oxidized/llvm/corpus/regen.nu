#!/usr/bin/env nu
# Regenerates the .ll conformance corpus.
#
# Two kinds of input feed it:
#
#   rustc/src/*.rs      compiled with `rustc --emit=llvm-ir` into rustc/*.ll
#   handwritten/*.ll    rewritten in place
#
# Both are then canonicalised by running them through real `llvm-as` and
# `llvm-dis`, so every committed file is exactly what upstream's own printer
# produces. That is what makes `llvm-roundtrip` a meaningful check: our
# printer has to match upstream byte for byte, not merely produce something
# that parses back. Canonicalising also strips the demangled-name comments
# rustc adds to `--emit=llvm-ir` output, which no IR printer reproduces.
#
# The corpus is committed, so no check ever shells out to rustc or to LLVM.
# Run this by hand after adding a seed, and record the tool versions in
# ../STATUS.md when they change.

const ORACLE_MAJOR = "21"

# The first line naming a version, falling back to the first line at all:
# `rustc --version` is one line without the word, `llvm-as --version` is three
# lines with it on the second.
def tool-version [name: string] {
  let lines = (^$name --version | lines | where ($it | is-not-empty))
  let named = ($lines | where ($it | str contains "version"))
  if ($named | is-empty) {
    $lines | first | str trim
  } else {
    $named | first | str trim
  }
}

# `llvm-as` then `llvm-dis` then `opt -S`, with the ModuleID comment
# normalised to the name the corpus file will have. Upstream sets that comment
# from whatever path it read, which would otherwise make the output depend on
# a temporary filename.
#
# The first two give the seed the compatibility upgrades the bitcode reader
# owes it, which is what a real pipeline would apply. The third is there
# because `llvm-dis` output is not a fixed point of the textual reader, and
# the textual reader is what this project implements: `opt -S` on a corpus
# file that came out of `llvm-dis` changes it. Across the whole corpus it
# changes twelve lines, all of them one rule, a `DICompositeType` with an
# identifier being made `distinct` as it is read. Leaving that out would ask
# our printer to reproduce output upstream's own `opt -S` does not.
def canonicalise [source: path, target: path, module_id: string] {
  let work = (mktemp -d)
  let bitcode = ([$work "module.bc"] | path join)
  let disassembled = ([$work "module.ll"] | path join)
  let printed = ([$work "fixed.ll"] | path join)
  ^llvm-as $source -o $bitcode
  ^llvm-dis $bitcode -o $disassembled
  ^opt -S $disassembled -o $printed
  let body = (open --raw $printed | lines)
  let head = $"; ModuleID = '($module_id)'"
  let rest = if ($body | first | str starts-with "; ModuleID") { $body | skip 1 } else { $body }
  ([$head ...$rest] | str join "\n") + "\n" | save -f $target
  rm -rf $work
}

# The `//@ flags:` line of a seed, if it has one.
def seed-flags [source: path]: nothing -> list<string> {
  let first = (open --raw $source | lines | first)
  if ($first | str starts-with "//@ flags:") {
    $first | str replace "//@ flags:" "" | str trim | split row " " | where ($it | is-not-empty)
  } else {
    []
  }
}

def main [] {
  let here = $env.FILE_PWD
  let oracle = (tool-version "llvm-as")
  if not ($oracle | str contains $" ($ORACLE_MAJOR).") {
    error make {msg: $"llvm-as is ($oracle), but the corpus is pinned to LLVM ($ORACLE_MAJOR)"}
  }
  print $"rustc:   (tool-version 'rustc')"
  print $"oracle:  ($oracle)"

  let seeds = (glob ([$here "rustc" "src" "*.rs"] | path join) | sort)
  for seed in $seeds {
    let stem = ($seed | path parse | get stem)
    let target = ([$here "rustc" $"($stem).ll"] | path join)
    let work = (mktemp -d)
    let raw = ([$work $"($stem).ll"] | path join)
    let flags = (seed-flags $seed)
    (^rustc --emit=llvm-ir --crate-type=lib -Cdebuginfo=0 -Copt-level=0
      --edition 2024 ...$flags -o $raw $seed)
    canonicalise $raw $target $"($stem).ll"
    rm -rf $work
    print $"  rustc/($stem).ll"
  }

  let handwritten = (glob ([$here "handwritten" "*.ll"] | path join) | sort)
  for file in $handwritten {
    let stem = ($file | path parse | get stem)
    canonicalise $file $file $"($stem).ll"
    print $"  handwritten/($stem).ll"
  }
}
