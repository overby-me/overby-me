#!/bin/bash
set -euo pipefail

# Test: unknown command produces error with "unknown command"

output=$($FLATPAK badcmd123 2>&1 || true)

if [ -z "$output" ]; then
  echo "FAIL: no output from unknown command"
  exit 1
fi

if echo "$output" | grep -qi "unknown command"; then
  echo "PASS: unknown command error message found"
else
  echo "FAIL: expected 'unknown command' in output, got:"
  echo "$output"
  exit 1
fi

# Also verify it exits non-zero
if $FLATPAK badcmd123 >/dev/null 2>&1; then
  echo "FAIL: expected non-zero exit code for unknown command"
  exit 1
fi

echo "PASS: unknown-command"