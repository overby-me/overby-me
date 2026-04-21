#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/urepo-app" org.test.URepo org.test.Sdk org.test.Platform
mkdir -p "$WORK/urepo-app/files/bin"
echo '#!/bin/sh' > "$WORK/urepo-app/files/bin/app"
$FLATPAK build-finish "$WORK/urepo-app" --command app
$FLATPAK build-export "$WORK/urepo" "$WORK/urepo-app"

rc=0
output=$($FLATPAK build-update-repo "$WORK/urepo" 2>&1) || rc=$?

if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
  echo "FAIL: build-update-repo crashed with signal (rc=$rc)"
  exit 1
fi

echo "PASS: build-update-repo"