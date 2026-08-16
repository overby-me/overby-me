#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user run } | complete)

if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit for 'run' with no args"
  exit 1
}

print "PASS: run-missing-args"
