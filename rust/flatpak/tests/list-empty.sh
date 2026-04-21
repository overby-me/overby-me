#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --user list 2>&1)

# Check it doesn't crash - if we got here, it didn't
# Check output contains header
echo "$output" | grep -qi "Name" || {
  echo "FAIL: list output does not contain 'Name' header: $output"
  exit 1
}

echo "PASS: list-empty"