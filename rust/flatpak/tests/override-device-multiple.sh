#!/bin/bash
set -euo pipefail

"$FLATPAK" --user override --device dri org.test.DevMulti
"$FLATPAK" --user override --device kvm org.test.DevMulti
"$FLATPAK" --user override --nodevice shm org.test.DevMulti

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.DevMulti"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "dri" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'dri'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "kvm" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'kvm'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "!shm" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain '!shm'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-device-multiple"