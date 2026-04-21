#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

# Add a remote first
"$FLATPAK" --user remote-add testremote https://example.com/repo

# Verify it was added
if ! "$FLATPAK" --user remotes 2>&1 | grep -q "testremote"; then
  echo "FAIL: testremote was not added"
  exit 1
fi

# Delete the remote
"$FLATPAK" --user remote-delete testremote

# Verify it was removed
if "$FLATPAK" --user remotes 2>&1 | grep -q "testremote"; then
  echo "FAIL: testremote still present after remote-delete"
  exit 1
fi

echo "PASS: remote-delete works correctly"