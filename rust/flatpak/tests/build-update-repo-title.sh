#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/title-app" org.test.Title org.test.Sdk org.test.Platform
mkdir -p "$WORK/title-app/files/bin"
echo '#!/bin/sh' > "$WORK/title-app/files/bin/app"
$FLATPAK build-finish "$WORK/title-app" --command app
$FLATPAK build-export "$WORK/title-repo" "$WORK/title-app" 2>&1

$FLATPAK build-update-repo --title="My Title" "$WORK/title-repo" 2>&1

grep -q "title=My Title" "$WORK/title-repo/config" || { echo "FAIL: title not set in repo config"; exit 1; }

echo "PASS: build-update-repo-title"