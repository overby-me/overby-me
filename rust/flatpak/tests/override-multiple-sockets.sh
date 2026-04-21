#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

"$FLATPAK" --user override --socket x11 org.test.Multi
"$FLATPAK" --user override --socket wayland org.test.Multi
"$FLATPAK" --user override --socket pulseaudio org.test.Multi

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Multi"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "x11" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'x11'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "wayland" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'wayland'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "pulseaudio" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'pulseaudio'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-multiple-sockets"