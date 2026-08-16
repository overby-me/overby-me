#!/usr/bin/env nu
source ./libtest-nix.nu

let r = (do { ^$env.FLATPAK --user run } | complete)
let rc = $r.exit_code
if $rc == 0 {
  print "FAIL: expected error for missing app"
  exit 1
}
ok "run missing args"

print "PASS: vm-run-missing-args"
