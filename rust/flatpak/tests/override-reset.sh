#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

# Create an override first
"$FLATPAK" --user override --socket wayland org.test.Hello

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Hello"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file was not created"
  exit 1
fi

echo "Override file exists after creation"

# Now reset (app_id must come before --reset due to arg parsing order)
"$FLATPAK" --user override org.test.Hello --reset

if [ -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file still exists after --reset"
  exit 1
fi

echo "PASS: override file removed after --reset"