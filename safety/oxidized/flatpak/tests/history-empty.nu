#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user history } | complete)

# Check it doesn't crash (segfault=139, abort=134, illegal=136)
let rc = $r.exit_code
if $rc == 139 or $rc == 134 or $rc == 136 {
  print "FAIL: flatpak history crashed"
  exit 1
}

print "PASS: history-empty"
