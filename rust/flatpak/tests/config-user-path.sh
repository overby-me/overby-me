#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --user config 2>&1) || true

if ! echo "$output" | grep -q "$HOME"; then
  echo "FAIL: config output does not contain HOME path '$HOME'"
  echo "$output"
  exit 1
fi

echo "PASS: config-user-path"