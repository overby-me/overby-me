#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

rc=0
output=$($FLATPAK --user run org.test.Nonexistent 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: expected non-zero exit for nonexistent app"; exit 1; }
ok "run nonexistent"

echo "PASS: vm-run-nonexistent"