#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user info } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}
let output = ($r.stdout + $r.stderr)
if not ($output =~ '(?i)specified|no application|usage') {
  print "FAIL: no hint"
  exit 1
}

print "PASS: info-missing-args"
