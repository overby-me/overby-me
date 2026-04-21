#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user mask org.test.Masked

# Check that a mask file was created somewhere under the installation
MASKS_DIR="$HOME/.local/share/flatpak"
if ! find "$MASKS_DIR" -type f -exec grep -l "org.test.Masked" {} + >/dev/null 2>&1; then
  echo "FAIL: mask for org.test.Masked not found under $MASKS_DIR"
  find "$MASKS_DIR" -type f | head -20
  exit 1
fi

echo "PASS: mask-pattern"