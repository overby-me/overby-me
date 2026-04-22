#!/bin/bash
set -euo pipefail
$FLATPAK build-init "$WORK/srtinfo" org.test.SRTInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/srtinfo/files/bin"
echo '#!/bin/sh' > "$WORK/srtinfo/files/bin/app"
chmod +x "$WORK/srtinfo/files/bin/app"
$FLATPAK build-finish "$WORK/srtinfo" --command app
$FLATPAK --user install "$WORK/srtinfo"

OUTPUT=$($FLATPAK --user info --show-runtime org.test.SRTInfo)
if echo "$OUTPUT" | grep -q "org.test.Platform"; then
    echo "PASS: info-show-runtime"
else
    echo "FAIL: info-show-runtime (expected 'org.test.Platform' in output, got '$OUTPUT')"
    exit 1
fi