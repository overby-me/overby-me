#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/deskapp" org.test.Desktop org.test.Sdk org.test.Platform
mkdir -p "$WORK/deskapp/files/share/applications"
cat > "$WORK/deskapp/files/share/applications/org.test.Desktop.desktop" << 'DESKTOP'
[Desktop Entry]
Name=Test
Exec=test
Type=Application
DESKTOP
mkdir -p "$WORK/deskapp/files/bin"
echo '#!/bin/sh' > "$WORK/deskapp/files/bin/test"
$FLATPAK build-finish "$WORK/deskapp" --command test
[ -f "$WORK/deskapp/export/share/applications/org.test.Desktop.desktop" ] || {
  echo "FAIL: desktop file not exported"
  exit 1
}

echo "PASS: build-finish-export-desktop"