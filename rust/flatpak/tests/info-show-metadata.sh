#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/metainfo" org.test.MetaInfo org.test.Sdk org.test.Platform
mkdir -p "$WORK/metainfo/files/bin"
echo '#!/bin/sh' > "$WORK/metainfo/files/bin/app"
chmod +x "$WORK/metainfo/files/bin/app"
$FLATPAK build-finish "$WORK/metainfo" --command app
$FLATPAK --user install "$WORK/metainfo"

output=$($FLATPAK --user info --show-metadata org.test.MetaInfo 2>&1)

if ! echo "$output" | grep -q "\[Application\]"; then
  echo "FAIL: output does not contain [Application]"
  echo "$output"
  exit 1
fi

if ! echo "$output" | grep -q "name=org.test.MetaInfo"; then
  echo "FAIL: output does not contain name=org.test.MetaInfo"
  echo "$output"
  exit 1
fi

echo "PASS: info-show-metadata"