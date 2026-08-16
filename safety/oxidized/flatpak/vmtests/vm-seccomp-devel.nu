#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

# Test that --devel mode works
let r1 = (do { ^$env.FLATPAK --user run --devel --command=sh org.test.Hello -c "echo devel-ok" } | complete)
let output1 = $r1.stdout + $r1.stderr
if ($output1 | lines | any {|l| $l =~ 'devel-ok' }) {
  ok "devel mode runs successfully"
} else {
  print "FAIL: expected 'devel-ok' in output"
  print $"Got: ($output1)"
  exit 1
}

# Test that mount is still blocked even in devel mode
let r2 = (do { ^$env.FLATPAK --user run --devel --command=sh org.test.Hello -c 'mount / /tmp -t tmpfs 2>&1 || echo BLOCKED' } | complete)
let output2 = $r2.stdout + $r2.stderr
if ($output2 | lines | any {|l| $l =~ 'BLOCKED|Operation not permitted|Permission denied' }) {
  ok "mount blocked in devel mode"
} else {
  print "FAIL: mount should be blocked even in devel mode"
  print $"Got: ($output2)"
  exit 1
}

print "PASS: vm-seccomp-devel"
