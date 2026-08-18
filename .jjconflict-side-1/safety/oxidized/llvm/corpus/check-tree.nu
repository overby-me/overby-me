#!/usr/bin/env nu
# Counts the modules we can read across a whole upstream test directory.
#
#   nu check-tree.nu <path-to-opt> <path-to-llvm-as> <directory> <ratchet>
#
# The `Assembler` and `Verifier` suites are written to exercise the parser and
# the verifier, which makes them a good conformance oracle and a poor sample of
# real IR. The rest of `llvm/test` is the opposite: tens of thousands of
# modules written to exercise passes, using whatever syntax was convenient at
# the time, including spellings that predate the ones LangRef documents.
#
# So this is the other bound. For every file the directory holds that real
# `llvm-as` reads, we have to read it too. It says nothing about printing it
# back or about verifying it, only that the module is not refused, which is the
# floor everything else sits on.
#
# The count only moves up. Unlike the two suite ratchets, there is no opposite
# bound to go with it, because there is nothing to trade against: reading a
# module upstream reads is right in every case.

def main [opt: path, llvm_as: path, suite: path, ratchet: int] {
  let files = (glob $"($suite)/**/*.ll" | sort)
  if ($files | is-empty) {
    error make {msg: $"no .ll files under ($suite)"}
  }

  mut readable = 0
  mut agreed = 0
  mut refused = []
  for file in $files {
    let upstream = (do { ^$llvm_as $file -o /dev/null } | complete)
    if $upstream.exit_code != 0 {
      continue
    }
    $readable = $readable + 1
    let ours = (do { ^$opt -S $file -o /dev/null } | complete)
    if $ours.exit_code == 0 {
      $agreed = $agreed + 1
    } else {
      $refused = ($refused | append $file)
    }
  }

  print $"($agreed) of ($readable) modules llvm-as reads, in ($files | length) files"
  if $agreed < $ratchet {
    print ($refused | each {|f| $f | path basename} | first 40 | str join " ")
    error make {msg: $"we read ($agreed), down from ($ratchet); that count only moves up"}
  }
  if $agreed > $ratchet {
    print $"agreement rose to ($agreed); raise the ratchet in default.nix to hold it"
  }
}
