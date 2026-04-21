#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/archinfo" org.test.ArchInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/archinfo/files/bin"
echo '#!/bin/sh' > "$WORK/archinfo/files/bin/app"
chmod +x "$WORK/archinfo/files/bin/app"
$FLATPAK build-finish "$WORK/archinfo" --command app
$FLATPAK --user install "$WORK/archinfo"

output=$($FLATPAK --user info org.test.ArchInfo 2>&1)

if ! echo "$output" | grep -qi "Arch"; then
  echo "FAIL: info output does not contain 'Arch'"
  echo "$output"
  exit 1
fi

echo "PASS: info-arch"