#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

"$FLATPAK" --user override --share network org.test.Hello
"$FLATPAK" --user override --unshare ipc org.test.Hello

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Hello"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not found at $OVERRIDE_FILE"
  exit 1
fi

echo "Override file contents:"
cat "$OVERRIDE_FILE"

if ! grep -qi "shared" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'shared' section"
  exit 1
fi

if ! grep -q "network" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'network'"
  exit 1
fi

if ! grep -q "!ipc" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain '!ipc'"
  exit 1
fi

echo "PASS: override share/unshare works correctly"