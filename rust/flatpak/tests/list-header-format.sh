#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --user list 2>&1)

if ! echo "$output" | head -1 | grep -q "Name"; then
  echo "FAIL: list header does not contain 'Name'"
  echo "$output"
  exit 1
fi

echo "PASS: list-header-format"