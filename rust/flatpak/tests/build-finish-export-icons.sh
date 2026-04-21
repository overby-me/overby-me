#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/iconapp" org.test.Icons org.test.Sdk org.test.Platform
mkdir -p "$WORK/iconapp/files/share/icons/hicolor/64x64/apps"
echo "PNG_DATA" > "$WORK/iconapp/files/share/icons/hicolor/64x64/apps/org.test.Icons.png"
mkdir -p "$WORK/iconapp/files/bin"
echo '#!/bin/sh' > "$WORK/iconapp/files/bin/test"
$FLATPAK build-finish "$WORK/iconapp" --command test
[ -f "$WORK/iconapp/export/share/icons/hicolor/64x64/apps/org.test.Icons.png" ]

echo "PASS: build-finish-export-icons"