#!/bin/bash
set -euo pipefail

output=$("$FLATPAK" --help 2>&1)

for cmd in install update uninstall list info run override search remotes; do
  if ! echo "$output" | grep -qi "$cmd"; then
    echo "FAIL: --help output does not mention '$cmd'"
    echo "$output"
    exit 1
  fi
done

echo "PASS: help-usage-format"