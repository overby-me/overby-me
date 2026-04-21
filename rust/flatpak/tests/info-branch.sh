#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/brinfo" org.test.BrInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/brinfo/files/bin"
echo '#!/bin/sh' > "$WORK/brinfo/files/bin/app"
chmod +x "$WORK/brinfo/files/bin/app"
$FLATPAK build-finish "$WORK/brinfo" --command app
$FLATPAK --user install "$WORK/brinfo"

output=$($FLATPAK --user info org.test.BrInfo 2>&1)

if ! echo "$output" | grep -qi "Branch"; then
  echo "FAIL: info output does not contain 'Branch'"
  echo "$output"
  exit 1
fi

echo "PASS: info-branch"