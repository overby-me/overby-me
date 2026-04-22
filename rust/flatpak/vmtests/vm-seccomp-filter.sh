#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

# Test that dangerous syscalls (mount) are blocked by seccomp filters
output=$(run_sh org.test.Hello "mount / /tmp -t tmpfs 2>&1 || echo BLOCKED" 2>&1)
echo "Seccomp test output: $output"

if echo "$output" | grep -qE "BLOCKED|Operation not permitted|Permission denied"; then
  ok "mount syscall is blocked by seccomp filter"
else
  echo "FAIL: mount syscall was not blocked in sandbox"
  echo "Got: $output"
  exit 1
fi

echo "PASS: vm-seccomp-filter"