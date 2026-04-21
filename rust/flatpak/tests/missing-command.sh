#!/bin/bash
set -euo pipefail

# Test: running flatpak with no args exits non-zero and shows usage
output=$("$FLATPAK" 2>&1 || true)
rc=0
"$FLATPAK" 2>/dev/null || rc=$?

if [ "$rc" -eq 0 ]; then
  echo "FAIL: expected non-zero exit code, got 0"
  exit 1
fi

if ! echo "$output" | grep -qi "Usage"; then
  echo "FAIL: output does not contain 'Usage'"
  echo "Got: $output"
  exit 1
fi

echo "PASS: missing command prints usage and exits non-zero (rc=$rc)"