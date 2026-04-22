#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/fa-app" org.test.FAInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/fa-app/files/bin"
echo '#!/bin/sh' > "$WORK/fa-app/files/bin/app"
chmod +x "$WORK/fa-app/files/bin/app"
$FLATPAK build-finish "$WORK/fa-app" --command app --filesystem home
$FLATPAK --user install "$WORK/fa-app"

# home should be read-write (granted via --filesystem home)
output=$($FLATPAK --user info --file-access=home org.test.FAInfo 2>&1)
if [ "$output" != "read-write" ]; then
  echo "FAIL: expected 'read-write' for home, got '$output'"
  exit 1
fi

# /usr should be hidden (not granted)
output=$($FLATPAK --user info --file-access=/usr org.test.FAInfo 2>&1)
if [ "$output" != "hidden" ]; then
  echo "FAIL: expected 'hidden' for /usr, got '$output'"
  exit 1
fi

echo "PASS: info-file-access"