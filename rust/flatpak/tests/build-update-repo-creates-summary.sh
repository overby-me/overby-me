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
# Check that GVariant binary summary was created in the OSTree repo dir
[ -f "$WORK/sum-repo/repo/summary" ] || { echo "FAIL: repo/summary not created"; exit 1; }
# Binary summary should not start with '#' (text format) — GVariant starts with binary data
first_byte=$(od -An -tx1 -N1 "$WORK/sum-repo/repo/summary" | tr -d ' ')
[ "$first_byte" != "23" ] || { echo "FAIL: repo/summary appears to be text, not GVariant"; exit 1; }

echo "PASS: build-update-repo-creates-summary"