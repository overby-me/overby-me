#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/uninstapp" org.test.Uninst org.test.Sdk org.test.Platform
mkdir -p "$WORK/uninstapp/files/bin"
echo '#!/bin/sh' > "$WORK/uninstapp/files/bin/app"
chmod +x "$WORK/uninstapp/files/bin/app"
$FLATPAK build-finish "$WORK/uninstapp" --command app
$FLATPAK --user install "$WORK/uninstapp"

# Verify it was installed
INSTALL_DIR="$HOME/.local/share/flatpak"
if ! find "$INSTALL_DIR" -type d -name "org.test.Uninst" 2>/dev/null | grep -q .; then
  echo "FAIL: app was not installed"
  exit 1
fi

$FLATPAK --user uninstall org.test.Uninst

# Check the deployment directory is gone
if find "$INSTALL_DIR/app/org.test.Uninst" -mindepth 1 -type d 2>/dev/null | grep -q .; then
  echo "FAIL: deployment directory still exists after uninstall"
  find "$INSTALL_DIR" -type d -name "org.test.Uninst" 2>/dev/null
  exit 1
fi

echo "PASS: uninstall-app"