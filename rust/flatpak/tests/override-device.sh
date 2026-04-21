#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

"$FLATPAK" --user override --device dri org.test.Hello
"$FLATPAK" --user override --nodevice kvm org.test.Hello

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Hello"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "devices" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'devices'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "dri" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'dri'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "!kvm" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain '!kvm'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-device"