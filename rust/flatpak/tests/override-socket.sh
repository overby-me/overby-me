#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

"$FLATPAK" --user override --socket wayland org.test.Hello
"$FLATPAK" --user override --nosocket ssh-auth org.test.Hello

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Hello"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -qi "sockets" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'sockets' section"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "wayland" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'wayland'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "!ssh-auth" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain '!ssh-auth'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-socket"