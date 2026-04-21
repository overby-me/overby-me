#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/datadir" org.test.DataDirs org.test.Sdk org.test.Platform
mkdir -p "$WORK/datadir/files/bin"
echo '#!/bin/sh' > "$WORK/datadir/files/bin/app"
$FLATPAK build-finish "$WORK/datadir" --command app
$FLATPAK --user install "$WORK/datadir"

# The installation should have the app directory structure
INST="$HOME/.local/share/flatpak"
APP_DIR=$(find "$INST/app" -maxdepth 1 -name "org.test.DataDirs" -type d 2>/dev/null)
[ -n "$APP_DIR" ] || { echo "FAIL: app directory not created"; exit 1; }

echo "PASS: install-creates-data-dirs"