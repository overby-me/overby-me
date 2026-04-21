#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user remote-add titled-remote https://example.com/repo --title="My Remote"

REMOTES_DIR="$HOME/.local/share/flatpak"

if ! find "$REMOTES_DIR" -type f -exec grep -l "titled-remote" {} + >/dev/null 2>&1; then
    echo "FAIL: 'titled-remote' not found in flatpak config under $REMOTES_DIR"
    exit 1
fi

if ! find "$REMOTES_DIR" -type f -exec grep -l "My Remote" {} + >/dev/null 2>&1; then
    echo "FAIL: 'My Remote' title not found in flatpak config under $REMOTES_DIR"
    find "$REMOTES_DIR" -type f -exec cat {} +
    exit 1
fi

echo "PASS: remote-add-title"