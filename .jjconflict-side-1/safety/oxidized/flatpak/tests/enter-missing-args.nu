#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK enter } | complete)
let rc = $r.exit_code

if $rc == 139 or $rc == 134 or $rc == 136 {
  print $"FAIL: enter crashed with signal \(rc=($rc)\)"
  exit 1
}

print "PASS: enter-missing-args"
