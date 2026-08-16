#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user install } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}
let output = ($r.stdout + $r.stderr)
if not ($output =~ '(?i)usage|specified|source') {
  print "FAIL: no usage hint"
  exit 1
}

print "PASS: install-missing-args"
