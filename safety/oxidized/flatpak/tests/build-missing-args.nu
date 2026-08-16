#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK build } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}
if ($r.stdout + $r.stderr) !~ '(?i)usage' {
  print "FAIL: expected usage hint in output"
  exit 1
}

print "PASS: build-missing-args"
