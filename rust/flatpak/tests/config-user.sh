#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --user config 2>&1) || true

if echo "$output" | grep -qi "user\|path"; then
  true
else
  echo "FAIL: config output does not contain 'user' or 'path'"
  echo "$output"
  exit 1
fi

echo "PASS: config-user"