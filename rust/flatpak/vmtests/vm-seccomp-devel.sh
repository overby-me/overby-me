#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

# Test that --devel mode works
output=$($FLATPAK --user run --devel --command=sh org.test.Hello -c "echo devel-ok" 2>&1)
if echo "$output" | grep -q "devel-ok"; then
  ok "devel mode runs successfully"
else
  echo "FAIL: expected 'devel-ok' in output"
  echo "Got: $output"
  exit 1
fi

# Test that mount is still blocked even in devel mode
output=$($FLATPAK --user run --devel --command=sh org.test.Hello -c "mount / /tmp -t tmpfs 2>&1 || echo BLOCKED" 2>&1)
if echo "$output" | grep -qE "BLOCKED|Operation not permitted|Permission denied"; then
  ok "mount blocked in devel mode"
else
  echo "FAIL: mount should be blocked even in devel mode"
  echo "Got: $output"
  exit 1
fi

echo "PASS: vm-seccomp-devel"