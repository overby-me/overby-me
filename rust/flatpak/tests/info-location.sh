#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/locinfo" org.test.LocInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/locinfo/files/bin"
echo '#!/bin/sh' > "$WORK/locinfo/files/bin/app"
chmod +x "$WORK/locinfo/files/bin/app"
$FLATPAK build-finish "$WORK/locinfo" --command app
$FLATPAK --user install "$WORK/locinfo"

output=$($FLATPAK --user info org.test.LocInfo 2>&1)

if ! echo "$output" | grep -qi "Location"; then
  echo "FAIL: info output does not contain 'Location'"
  echo "$output"
  exit 1
fi

echo "PASS: info-location"