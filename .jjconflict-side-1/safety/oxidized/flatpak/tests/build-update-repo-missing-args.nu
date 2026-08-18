#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK build-update-repo } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}

print "PASS: build-update-repo-missing-args"
