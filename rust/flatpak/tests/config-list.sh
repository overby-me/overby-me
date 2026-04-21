#!/bin/bash
set -euo pipefail

# Test: flatpak --user config prints something useful

mkdir -p "$HOME/.local/share/flatpak"

output=$("$FLATPAK" --user config 2>&1 || true)

if [ -z "$output" ]; then
  echo "FAIL: flatpak --user config produced no output"
  exit 1
fi

echo "Output: $output"
echo "PASS: flatpak --user config produced output"