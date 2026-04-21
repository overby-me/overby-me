#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user remote-add modremote https://example.com/v1
$FLATPAK --user remote-delete modremote
$FLATPAK --user remote-add modremote https://example.com/v2

REMOTES_DIR="$HOME/.local/share/flatpak"
if ! find "$REMOTES_DIR" -type f -exec grep -l "https://example.com/v2" {} + >/dev/null 2>&1; then
  echo "FAIL: v2 URL not found in flatpak config"
  exit 1
fi

if find "$REMOTES_DIR" -type f -exec grep -l "https://example.com/v1" {} + >/dev/null 2>&1; then
  echo "FAIL: old v1 URL still present in flatpak config"
  exit 1
fi

echo "PASS: remote-modify-implicit"