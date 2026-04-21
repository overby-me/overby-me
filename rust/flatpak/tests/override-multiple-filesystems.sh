#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user override --filesystem home org.test.Multi
$FLATPAK --user override --filesystem /tmp org.test.Multi
$FLATPAK --user override --filesystem xdg-desktop org.test.Multi

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Multi"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "home" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'home'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "/tmp" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain '/tmp'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "xdg-desktop" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'xdg-desktop'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-multiple-filesystems"