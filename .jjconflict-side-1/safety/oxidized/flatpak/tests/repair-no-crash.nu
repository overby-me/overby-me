#!/usr/bin/env nu

let _output = (do { ^$env.FLATPAK --user repair } | complete)

# Check it didn't crash with a signal (segfault=139, abort=134, illegal=136)
let rc = (do { ^$env.FLATPAK --user repair } | complete).exit_code
if $rc in [139 134 136] {
  print $"FAIL: repair crashed with signal \(exit code ($rc)\)"
  exit 1
}

print "PASS: repair-no-crash"
