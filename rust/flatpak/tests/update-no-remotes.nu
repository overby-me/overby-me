#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user update } | complete)
let rc = $r.exit_code
if $rc in [139 134 136] {
  print $"FAIL: update crashed with signal \(rc=($rc)\)"
  exit 1
}

print "PASS: update-no-remotes"
