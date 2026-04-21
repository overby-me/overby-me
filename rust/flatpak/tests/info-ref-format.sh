#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/refinfo" org.test.RefFmt org.test.Sdk org.test.Platform
mkdir -p "$WORK/refinfo/files/bin"
echo '#!/bin/sh' > "$WORK/refinfo/files/bin/app"
$FLATPAK build-finish "$WORK/refinfo" --command app
$FLATPAK --user install "$WORK/refinfo"
output=$($FLATPAK --user info org.test.RefFmt 2>&1)
echo "$output" | grep -q "Ref"
echo "$output" | grep -q "app/"
echo "$output" | grep -q "org.test.RefFmt"

echo "PASS: info-ref-format"