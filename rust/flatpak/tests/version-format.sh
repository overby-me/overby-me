#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --version 2>&1)

if ! echo "$output" | grep -qP 'Flatpak \d+\.\d+\.\d+'; then
  echo "FAIL: version output does not match 'Flatpak X.Y.Z' pattern: $output"
  exit 1
fi

echo "PASS: version-format"