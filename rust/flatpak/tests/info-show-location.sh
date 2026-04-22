#!/bin/bash
set -euo pipefail
$FLATPAK build-init "$WORK/slocinfo" org.test.SLocInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/slocinfo/files/bin"
echo '#!/bin/sh' > "$WORK/slocinfo/files/bin/app"
chmod +x "$WORK/slocinfo/files/bin/app"
$FLATPAK build-finish "$WORK/slocinfo" --command app
$FLATPAK --user install "$WORK/slocinfo"
output=$($FLATPAK --user info --show-location org.test.SLocInfo)
if echo "$output" | grep -q "org.test.SLocInfo"; then
    echo "PASS: info-show-location"
else
    echo "FAIL: info-show-location (expected path containing org.test.SLocInfo, got: $output)"
    exit 1
fi