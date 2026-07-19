#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --help } | complete)
let output = ($r.stdout + $r.stderr)
if not ($output | str contains "Usage:") {
  print "FAIL: --help output missing 'Usage:'"
  exit 1
}
print "PASS: --help contains 'Usage:'"
