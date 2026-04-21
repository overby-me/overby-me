#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

"$FLATPAK" --user remote-add myremote https://example.com/repo

output=$("$FLATPAK" --user remotes 2>&1)

if ! echo "$output" | grep -q "myremote"; then
  echo "FAIL: remotes output does not contain 'myremote'"
  echo "Output was: $output"
  exit 1
fi

echo "PASS: remote-list"