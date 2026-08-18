#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

let r = (do { run org.test.Hello } | complete)
let output = $r.stdout + $r.stderr
if not ($output | lines | any {|l| $l =~ 'Hello world, from a sandbox' }) {
  print "FAIL: expected 'Hello world, from a sandbox' in output"
  print $"Got: ($output)"
  exit 1
}

ok "run hello"
print "PASS: vm-run-hello"
