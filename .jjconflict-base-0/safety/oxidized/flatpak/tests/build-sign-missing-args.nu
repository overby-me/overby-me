#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK build-sign } | complete)
let rc = $r.exit_code

# It may not error, just check it doesn't crash with a signal
if $rc == 139 or $rc == 134 or $rc == 136 {
  print $"FAIL: build-sign crashed with signal \(rc=($rc)\)"
  exit 1
}

print "PASS: build-sign-missing-args"
