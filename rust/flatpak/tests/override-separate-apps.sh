#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user override --socket x11 org.test.App1
$FLATPAK --user override --socket wayland org.test.App2

OVERRIDE1="$HOME/.local/share/flatpak/overrides/org.test.App1"
OVERRIDE2="$HOME/.local/share/flatpak/overrides/org.test.App2"

if [ ! -f "$OVERRIDE1" ]; then
  echo "FAIL: override file not created for org.test.App1"
  exit 1
fi

if [ ! -f "$OVERRIDE2" ]; then
  echo "FAIL: override file not created for org.test.App2"
  exit 1
fi

if ! grep -q "x11" "$OVERRIDE1"; then
  echo "FAIL: org.test.App1 override does not contain 'x11'"
  cat "$OVERRIDE1"
  exit 1
fi

if grep -q "wayland" "$OVERRIDE1"; then
  echo "FAIL: org.test.App1 override should not contain 'wayland'"
  cat "$OVERRIDE1"
  exit 1
fi

if ! grep -q "wayland" "$OVERRIDE2"; then
  echo "FAIL: org.test.App2 override does not contain 'wayland'"
  cat "$OVERRIDE2"
  exit 1
fi

if grep -q "x11" "$OVERRIDE2"; then
  echo "FAIL: org.test.App2 override should not contain 'x11'"
  cat "$OVERRIDE2"
  exit 1
fi

echo "PASS: override-separate-apps"