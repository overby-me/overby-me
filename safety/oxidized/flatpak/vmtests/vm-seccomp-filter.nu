#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

# Test that dangerous syscalls (mount) are blocked by seccomp filters
let r = (do { run_sh org.test.Hello 'mount / /tmp -t tmpfs 2>&1 || echo BLOCKED' } | complete)
let output = $r.stdout + $r.stderr
print $"Seccomp test output: ($output)"

if ($output | lines | any {|l| $l =~ 'BLOCKED|Operation not permitted|Permission denied' }) {
  ok "mount syscall is blocked by seccomp filter"
} else {
  print "FAIL: mount syscall was not blocked in sandbox"
  print $"Got: ($output)"
  exit 1
}

print "PASS: vm-seccomp-filter"
