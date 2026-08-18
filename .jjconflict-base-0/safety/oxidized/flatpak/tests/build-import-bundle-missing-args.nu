#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK build-import-bundle } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}

print "PASS: build-import-bundle-missing-args"
