#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/rtfilt" org.test.RTFilt org.test.Sdk org.test.Platform
mkdir -p "$WORK/rtfilt/files/bin"
echo '#!/bin/sh' > "$WORK/rtfilt/files/bin/app"
$FLATPAK build-finish "$WORK/rtfilt" --command app
$FLATPAK --user install "$WORK/rtfilt"
$FLATPAK --user list --runtime > "$WORK/rtout"
if grep -q "org.test.RTFilt" "$WORK/rtout"; then
  echo "FAIL: app should not appear in --runtime list"
  exit 1
fi
$FLATPAK --user list --app > "$WORK/appout"
grep -q "org.test.RTFilt" "$WORK/appout"

echo "PASS: list-runtime-filter"