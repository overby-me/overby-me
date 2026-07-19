#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
let r = (do { run "--command" sh org.test.Hello "-c" "echo custom-command" } | complete)
let output = $r.stdout + $r.stderr
if not ($output | lines | any {|l| $l =~ 'custom-command' }) {
  print "FAIL: --command override didn't work"
  print $"Got: ($output)"
  exit 1
}
ok "run command override"

print "PASS: vm-run-command-override"
