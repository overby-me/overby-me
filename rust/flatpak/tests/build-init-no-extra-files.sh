#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/cleanapp" org.test.Clean org.test.Sdk org.test.Platform
# Expected: metadata, files/, var/
[ -f "$WORK/cleanapp/metadata" ]
[ -d "$WORK/cleanapp/files" ]
[ -d "$WORK/cleanapp/var" ]
# No other top-level items besides metadata, files, var
count=$(ls "$WORK/cleanapp" | wc -l)
[ "$count" -le 3 ] || { echo "FAIL: unexpected files in build dir: $(ls $WORK/cleanapp)"; exit 1; }

echo "PASS: build-init-no-extra-files"