#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/deldata" org.test.DelData org.test.Sdk org.test.Platform
mkdir -p "$WORK/deldata/files/bin"
echo '#!/bin/sh' > "$WORK/deldata/files/bin/app"
$FLATPAK build-finish "$WORK/deldata" --command app
$FLATPAK --user install "$WORK/deldata"

# Create app data
mkdir -p "$HOME/.var/app/org.test.DelData/data"
echo "userdata" > "$HOME/.var/app/org.test.DelData/data/file.txt"

# Uninstall without --delete-data: data should remain
$FLATPAK --user uninstall org.test.DelData
[ -f "$HOME/.var/app/org.test.DelData/data/file.txt" ] || { echo "FAIL: data deleted without --delete-data"; exit 1; }

# Re-install and uninstall with --delete-data
$FLATPAK --user install "$WORK/deldata"
$FLATPAK --user uninstall --delete-data org.test.DelData
if [ -d "$HOME/.var/app/org.test.DelData" ]; then
  echo "FAIL: data dir still exists after --delete-data"
  exit 1
fi

echo "PASS: uninstall-delete-data"