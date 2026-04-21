#!/bin/bash
set -euo pipefail

"$FLATPAK" --user override --socket x11 --device dri --share network org.test.IniFormat

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.IniFormat"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "^\[Context\]" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain [Context] group header"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "sockets=.*x11" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'sockets=.*x11'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "devices=.*dri" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'devices=.*dri'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "shared=.*network" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'shared=.*network'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-file-format"