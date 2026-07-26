#!/usr/bin/env nu
# Drives the real `opt` binary over every corpus file and requires the output
# to match the input byte for byte.
#
# The cargo test does the same thing in process; this one exists because a
# tool that is only ever exercised through its library is a tool nobody has
# run. It catches argument handling, file writing and exit codes as well as
# the printer.
#
#   nu check-opt.nu <path-to-opt> <corpus-directory>

def main [opt: path, corpus: path] {
  let files = (glob ([$corpus "**" "*.ll"] | path join) | sort)
  if ($files | is-empty) {
    error make {msg: $"no .ll files under ($corpus)"}
  }

  let work = (mktemp -d)
  mut failures = []
  for file in $files {
    let out = ([$work "out.ll"] | path join)
    let result = (do { ^$opt -S -passes=verify $file -o $out } | complete)
    if $result.exit_code != 0 {
      $failures = ($failures | append $"($file | path basename): opt exited ($result.exit_code): ($result.stderr | str trim)")
      continue
    }
    if (open --raw $file) != (open --raw $out) {
      $failures = ($failures | append $"($file | path basename): output differs from input")
    }
  }
  rm -rf $work

  if ($failures | is-not-empty) {
    print ($failures | str join "\n")
    error make {msg: $"($failures | length) of ($files | length) corpus files did not survive opt"}
  }
  print $"opt reproduced all ($files | length) corpus files exactly"
}
