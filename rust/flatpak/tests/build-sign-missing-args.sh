#!/bin/bash
set -euo pipefail

rc=0
output=$($FLATPAK build-sign 2>&1) || rc=$?

# It may not error, just check it doesn't crash with a signal
if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
  echo "FAIL: build-sign crashed with signal (rc=$rc)"
  exit 1
fi

echo "PASS: build-sign-missing-args"