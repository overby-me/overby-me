#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK search nonexistent } | complete)

# Check it doesn't crash (segfault=139, abort=134, illegal=136)
if $r.exit_code in [139 134 136] {
  print "FAIL: flatpak search crashed"
  exit 1
}

print "PASS: search-no-remote"
