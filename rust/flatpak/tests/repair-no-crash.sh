#!/bin/bash
set -euo pipefail

output=$($FLATPAK --user repair 2>&1 || true)

# Check it didn't crash with a signal (segfault=139, abort=134, illegal=136)
rc=0
$FLATPAK --user repair 2>&1 || rc=$?
if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
  echo "FAIL: repair crashed with signal (exit code $rc)"
  exit 1
fi

echo "PASS: repair-no-crash"