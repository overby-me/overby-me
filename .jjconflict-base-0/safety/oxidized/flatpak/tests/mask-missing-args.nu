#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user mask } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}

print "PASS: mask-missing-args"
