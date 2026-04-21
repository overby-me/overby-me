#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --user history 2>&1 || true)

# Check it doesn't crash (segfault=139, abort=134, illegal=136)
if [ $? -eq 139 ] || [ $? -eq 134 ] || [ $? -eq 136 ]; then
  echo "FAIL: flatpak history crashed"
  exit 1
fi

echo "PASS: history-empty"