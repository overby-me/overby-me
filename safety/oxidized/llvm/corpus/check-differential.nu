#!/usr/bin/env nu
# Compares what we print against what upstream prints, for every file in a
# suite that both of us accept.
#
#   nu check-differential.nu <path-to-opt> <suite-directory> <ratchet>
#
# The corpus round trip already pins the printer, but only against files that
# came out of upstream's printer in the first place. This runs the comparison
# the other way round, over inputs nobody wrote for us: hand-written upstream
# tests full of syntax the corpus never contains. A file counts as matching
# when our `opt -S` output equals upstream's `opt -S` output line for line.
#
# Upstream's `opt -S` and not `llvm-as | llvm-dis`, because the latter is a
# different transformation: writing bitcode and reading it back applies the
# compatibility upgrades the bitcode reader owes older files, and those are
# not things a text-to-text printer does. The clearest case is the data
# layout, which the bitcode reader rewrites for the target the triple names
# and the textual reader leaves alone: `target datalayout = "e"` with an
# x86 triple comes back as `"e-i128:128"` through bitcode and as `"e"`
# through `opt -S`, which is what we print. Thirteen files differed for that
# reason alone and none of them was a disagreement about printing.
#
# Two path-derived lines are dropped from both sides before comparing. We keep
# the ModuleID the input carried and upstream regenerates it from whichever
# path it read; and upstream synthesises a source_filename from the input path
# when the file has none, while we leave the field empty. Neither line says
# anything about agreement, and both would otherwise make every comparison
# fail. The corpus round trip pins both fields properly, against files that
# carry them. Everything else is a real difference.
#
# The ratchet only ever moves up.

def canonical [text: string]: nothing -> string {
  $text
  | lines
  | where not (($it | str starts-with "; ModuleID") or ($it | str starts-with "source_filename ="))
  | str join "\n"
}

def main [opt: path, upstream_opt: path, suite: path, ratchet: int] {
  let files = (glob ([$suite "*.ll"] | path join) | sort)
  if ($files | is-empty) {
    error make {msg: $"no .ll files in ($suite)"}
  }

  let work = (mktemp -d)
  mut matched = 0
  mut considered = 0
  mut differences = []
  for file in $files {
    let printed = (do { ^$upstream_opt -S $file -o - } | complete)
    if $printed.exit_code != 0 {
      # Upstream rejects it, so there is nothing to compare against.
      continue
    }
    let ours = (do { ^$opt -S -passes=verify $file -o - } | complete)
    if $ours.exit_code != 0 {
      # Counted by check-upstream.nu as a disagreement; not a print
      # difference, so it is out of scope here.
      continue
    }
    $considered = $considered + 1
    if (canonical $ours.stdout) == (canonical $printed.stdout) {
      $matched = $matched + 1
    } else {
      $differences = ($differences | append ($file | path basename))
    }
  }
  rm -rf $work

  print $"suite:     ($suite | path basename)"
  print $"identical: ($matched) of ($considered) files we both accept"
  print $"ratchet:   ($ratchet)"
  if ($differences | is-not-empty) {
    print ""
    print "printed differently:"
    print ($differences | first 30 | str join "\n")
    if (($differences | length) > 30) {
      print $"... and (($differences | length) - 30) more"
    }
  }

  if $matched < $ratchet {
    error make {msg: $"agreement fell from ($ratchet) to ($matched); the ratchet only moves up"}
  }
  if $matched > $ratchet {
    print ""
    print $"agreement rose to ($matched); raise the ratchet in default.nix to hold it"
  }
}
