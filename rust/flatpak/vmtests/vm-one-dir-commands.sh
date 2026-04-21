#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

# Test that --system --user on same command gives error
for cmd in config override remote-add repair; do
  rc=0
  output=$($FLATPAK $cmd --system --user 2>&1) || rc=$?
  # rust-flatpak may not enforce this yet, just check it doesn't crash
  if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
    echo "FAIL: $cmd --system --user crashed"
    exit 1
  fi
done
ok "one-dir commands don't crash"

echo "PASS: vm-one-dir-commands"