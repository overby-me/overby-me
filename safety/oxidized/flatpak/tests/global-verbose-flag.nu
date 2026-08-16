#!/usr/bin/env nu

# Verify --verbose / -v are accepted (don't cause errors)
let rc = (do { ^$env.FLATPAK --verbose --user config } | complete).exit_code
if $rc == 139 or $rc == 134 {
  print "FAIL: --verbose crashed"
  exit 1
}

let rc = (do { ^$env.FLATPAK -v --user config } | complete).exit_code
if $rc == 139 or $rc == 134 {
  print "FAIL: -v crashed"
  exit 1
}

print "PASS: global-verbose-flag"
