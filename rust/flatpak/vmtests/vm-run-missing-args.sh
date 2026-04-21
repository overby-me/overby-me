#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

rc=0
output=$($FLATPAK --user run 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: expected error for missing app"; exit 1; }
ok "run missing args"

echo "PASS: vm-run-missing-args"