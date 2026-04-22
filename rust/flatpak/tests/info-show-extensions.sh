#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/ext-app" org.test.ExtInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/ext-app/files/bin"
echo '#!/bin/sh' > "$WORK/ext-app/files/bin/app"
chmod +x "$WORK/ext-app/files/bin/app"
$FLATPAK build-finish "$WORK/ext-app" --command app
$FLATPAK --user install "$WORK/ext-app"

output=$($FLATPAK --user info --show-extensions org.test.ExtInfo 2>&1)

if echo "$output" | grep -qi "No extensions"; then
  true
else
  echo "FAIL: expected 'No extensions' for app without extension points"
  echo "Got: $output"
  exit 1
fi

echo "PASS: info-show-extensions"