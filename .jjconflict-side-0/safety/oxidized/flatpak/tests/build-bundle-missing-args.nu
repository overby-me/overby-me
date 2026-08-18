#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK build-bundle } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}

print "PASS: build-bundle-missing-args"
