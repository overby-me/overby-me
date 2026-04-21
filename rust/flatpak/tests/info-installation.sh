#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/instinfo" org.test.InstInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/instinfo/files/bin"
echo '#!/bin/sh' > "$WORK/instinfo/files/bin/app"
chmod +x "$WORK/instinfo/files/bin/app"
$FLATPAK build-finish "$WORK/instinfo" --command app
$FLATPAK --user install "$WORK/instinfo"

output=$($FLATPAK --user info org.test.InstInfo 2>&1)

if ! echo "$output" | grep -q "user"; then
  echo "FAIL: info output does not contain 'user' installation"
  echo "$output"
  exit 1
fi

echo "PASS: info-installation"