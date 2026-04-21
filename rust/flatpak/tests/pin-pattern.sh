#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user pin org.test.Pinned

# Check that a pin file was created somewhere under the installation
PINS_DIR="$HOME/.local/share/flatpak"
if ! find "$PINS_DIR" -type f -exec grep -l "org.test.Pinned" {} + >/dev/null 2>&1; then
  echo "FAIL: pin for org.test.Pinned not found under $PINS_DIR"
  find "$PINS_DIR" -type f | head -20
  exit 1
fi

echo "PASS: pin-pattern"