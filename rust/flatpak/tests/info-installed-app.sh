#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/infoapp" org.test.Info org.test.Sdk org.test.Platform
mkdir -p "$WORK/infoapp/files/bin"
echo '#!/bin/sh' > "$WORK/infoapp/files/bin/app"
chmod +x "$WORK/infoapp/files/bin/app"
$FLATPAK build-finish "$WORK/infoapp" --command app
$FLATPAK --user install "$WORK/infoapp"

output=$($FLATPAK --user info org.test.Info 2>&1)

if ! echo "$output" | grep -q "org.test.Info"; then
  echo "FAIL: info output does not contain 'org.test.Info'"
  echo "$output"
  exit 1
fi

echo "PASS: info-installed-app"