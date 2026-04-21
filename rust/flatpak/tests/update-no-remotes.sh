#!/bin/bash
set -euo pipefail

rc=0
output=$($FLATPAK --user update 2>&1) || rc=$?
if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
  echo "FAIL: update crashed with signal (rc=$rc)"
  exit 1
fi

echo "PASS: update-no-remotes"