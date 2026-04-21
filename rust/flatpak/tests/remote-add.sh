#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user remote-add testremote https://example.com/repo

# Check that the remote was added somewhere in the flatpak config
REMOTES_DIR="$HOME/.local/share/flatpak"
if ! find "$REMOTES_DIR" -type f -exec grep -l "testremote" {} + >/dev/null 2>&1; then
    echo "FAIL: 'testremote' not found in flatpak config under $REMOTES_DIR"
    exit 1
fi

if ! find "$REMOTES_DIR" -type f -exec grep -l "https://example.com/repo" {} + >/dev/null 2>&1; then
    echo "FAIL: 'https://example.com/repo' not found in flatpak config under $REMOTES_DIR"
    exit 1
fi

echo "PASS: remote-add created testremote with correct URL"