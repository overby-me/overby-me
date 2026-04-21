#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/rtinfo" org.test.RTInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/rtinfo/files/bin"
echo '#!/bin/sh' > "$WORK/rtinfo/files/bin/app"
chmod +x "$WORK/rtinfo/files/bin/app"
$FLATPAK build-finish "$WORK/rtinfo" --command app
$FLATPAK --user install "$WORK/rtinfo"

output=$($FLATPAK --user info org.test.RTInfo 2>&1)

if ! echo "$output" | grep -qi "Runtime"; then
  echo "FAIL: info output does not contain 'Runtime'"
  echo "$output"
  exit 1
fi

if ! echo "$output" | grep -q "org.test.Platform"; then
  echo "FAIL: info output does not contain 'org.test.Platform'"
  echo "$output"
  exit 1
fi

echo "PASS: info-runtime"