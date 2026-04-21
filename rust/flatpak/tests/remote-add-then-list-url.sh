#!/bin/bash
set -euo pipefail

$FLATPAK --user remote-add urlcheck https://url.example.com/flatpak

# Check the config file directly for the URL
if ! find "$HOME/.local/share/flatpak" -type f | xargs grep -l "url.example.com" > /dev/null 2>&1; then
  echo "FAIL: could not find url.example.com in any flatpak config file"
  find "$HOME/.local/share/flatpak" -type f 2>/dev/null || true
  exit 1
fi

echo "PASS: remote-add-then-list-url"