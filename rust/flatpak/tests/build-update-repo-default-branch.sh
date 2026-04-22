#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/defbranch-app" org.test.DefBranch org.test.Sdk org.test.Platform
mkdir -p "$WORK/defbranch-app/files/bin"
echo '#!/bin/sh' > "$WORK/defbranch-app/files/bin/app"
$FLATPAK build-finish "$WORK/defbranch-app" --command app
$FLATPAK build-export "$WORK/defbranch-repo" "$WORK/defbranch-app" 2>&1
$FLATPAK build-update-repo --default-branch=beta "$WORK/defbranch-repo" 2>&1

grep -q "default-branch=beta" "$WORK/defbranch-repo/config" || { echo "FAIL: default-branch not set in repo config"; exit 1; }

echo "PASS: build-update-repo-default-branch"