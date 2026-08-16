#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK build-finish } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}
if ($r.stdout + $r.stderr) !~ '(?i)usage' {
  print "FAIL: no usage hint"
  exit 1
}

print "PASS: build-finish-no-dir"
