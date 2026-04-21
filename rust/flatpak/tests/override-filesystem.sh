#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

"$FLATPAK" --user override --filesystem home org.test.Hello

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Hello"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -qi "filesystems" "$OVERRIDE_FILE"; then
  echo "FAIL: override file missing 'filesystems' key"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "home" "$OVERRIDE_FILE"; then
  echo "FAIL: override file missing 'home' filesystem"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-filesystem"