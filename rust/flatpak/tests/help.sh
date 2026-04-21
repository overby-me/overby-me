#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --help 2>&1)
echo "$output" | grep -q "Usage:" || { echo "FAIL: --help output missing 'Usage:'"; exit 1; }
echo "PASS: --help contains 'Usage:'"