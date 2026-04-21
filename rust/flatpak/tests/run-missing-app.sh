#!/bin/bash
set -euo pipefail

rc=0
output=$("$FLATPAK" --user run org.test.NonExistent 2>&1) || rc=$?

if [ "$rc" -eq 0 ]; then
  echo "FAIL: expected non-zero exit for missing app"
  echo "$output"
  exit 1
fi

echo "PASS: run-missing-app"