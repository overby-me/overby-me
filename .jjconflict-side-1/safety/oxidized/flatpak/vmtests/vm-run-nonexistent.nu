#!/usr/bin/env nu
source ./libtest-nix.nu

let r = (do { ^$env.FLATPAK --user run org.test.Nonexistent } | complete)
let rc = $r.exit_code
if $rc == 0 {
  print "FAIL: expected non-zero exit for nonexistent app"
  exit 1
}
ok "run nonexistent"

print "PASS: vm-run-nonexistent"
