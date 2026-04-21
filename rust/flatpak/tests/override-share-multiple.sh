#!/bin/bash
set -euo pipefail

"$FLATPAK" --user override --share network org.test.ShareMulti
"$FLATPAK" --user override --share ipc org.test.ShareMulti
"$FLATPAK" --user override --unshare cups org.test.ShareMulti

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.ShareMulti"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "network" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'network'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "ipc" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'ipc'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "!cups" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain '!cups'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-share-multiple"