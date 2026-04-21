#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --version 2>&1)
echo "$output" | grep -qi "flatpak" || {
  echo "FAIL: version output does not contain 'flatpak': $output"
  exit 1
}
echo "PASS: version output contains 'flatpak'"