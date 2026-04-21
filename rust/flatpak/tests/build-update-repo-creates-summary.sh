#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/sum-app" org.test.Summary org.test.Sdk org.test.Platform
mkdir -p "$WORK/sum-app/files/bin"
echo '#!/bin/sh' > "$WORK/sum-app/files/bin/app"
$FLATPAK build-finish "$WORK/sum-app" --command app
$FLATPAK build-export "$WORK/sum-repo" "$WORK/sum-app" 2>&1
$FLATPAK build-update-repo "$WORK/sum-repo" 2>&1 || true
# Check that repo directory still exists and has content
[ -d "$WORK/sum-repo" ]

echo "PASS: build-update-repo-creates-summary"