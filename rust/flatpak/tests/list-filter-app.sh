#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/filterapp" org.test.Listed org.test.Sdk org.test.Platform
mkdir -p "$WORK/filterapp/files/bin"
echo '#!/bin/sh' > "$WORK/filterapp/files/bin/app"
chmod +x "$WORK/filterapp/files/bin/app"
$FLATPAK build-finish "$WORK/filterapp" --command app
$FLATPAK --user install "$WORK/filterapp"

$FLATPAK --user list --app > "$WORK/applist"
$FLATPAK --user list --runtime > "$WORK/rtlist"

if ! grep -q "org.test.Listed" "$WORK/applist"; then
  echo "FAIL: --app list does not contain org.test.Listed"
  cat "$WORK/applist"
  exit 1
fi

if grep -q "org.test.Listed" "$WORK/rtlist"; then
  echo "FAIL: --runtime list should not contain org.test.Listed"
  cat "$WORK/rtlist"
  exit 1
fi

echo "PASS: list-filter-app"