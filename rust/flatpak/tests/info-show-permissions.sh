#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/perminfo" org.test.PermInfo org.test.Sdk org.test.Platform
mkdir -p "$WORK/perminfo/files/bin"
echo '#!/bin/sh' > "$WORK/perminfo/files/bin/app"
chmod +x "$WORK/perminfo/files/bin/app"
$FLATPAK build-finish "$WORK/perminfo" --command app --share network --socket x11
$FLATPAK --user install "$WORK/perminfo"

$FLATPAK --user info --show-permissions org.test.PermInfo > "$WORK/perm_out"

if ! grep -q "network" "$WORK/perm_out"; then
  echo "FAIL: --show-permissions output does not contain 'network'"
  cat "$WORK/perm_out"
  exit 1
fi

if ! grep -q "x11" "$WORK/perm_out"; then
  echo "FAIL: --show-permissions output does not contain 'x11'"
  cat "$WORK/perm_out"
  exit 1
fi

echo "PASS: info-show-permissions"