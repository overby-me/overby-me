#!/bin/bash
set -euo pipefail

rc=0
output=$($FLATPAK kill 2>&1) || rc=$?

if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
  echo "FAIL: kill crashed with signal (rc=$rc)"
  exit 1
fi

echo "PASS: kill-missing-args"