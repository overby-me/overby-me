#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/ssdkinfo" org.test.SSdkInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/ssdkinfo/files/bin"
echo '#!/bin/sh' > "$WORK/ssdkinfo/files/bin/app"
chmod +x "$WORK/ssdkinfo/files/bin/app"
$FLATPAK build-finish "$WORK/ssdkinfo" --command app --sdk=org.test.Sdk/x86_64/stable
$FLATPAK --user install "$WORK/ssdkinfo"

output=$($FLATPAK --user info --show-sdk org.test.SSdkInfo)
if echo "$output" | grep -q "org.test.Sdk"; then
    echo "PASS: info-show-sdk"
else
    echo "FAIL: info-show-sdk (expected 'org.test.Sdk' in output, got: $output)"
    exit 1
fi