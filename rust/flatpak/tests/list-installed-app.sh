#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/listapp" org.test.Listed org.test.Sdk org.test.Platform
mkdir -p "$WORK/listapp/files/bin"
echo '#!/bin/sh' > "$WORK/listapp/files/bin/app"
chmod +x "$WORK/listapp/files/bin/app"
$FLATPAK build-finish "$WORK/listapp" --command app
$FLATPAK --user install "$WORK/listapp"

output=$($FLATPAK --user list 2>&1)

if ! echo "$output" | grep -q "org.test.Listed"; then
  echo "FAIL: list output does not contain 'org.test.Listed'"
  echo "$output"
  exit 1
fi

echo "PASS: list-installed-app"