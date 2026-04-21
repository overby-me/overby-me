#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/cmdinfo" org.test.CmdInfo org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/cmdinfo/files/bin"
echo '#!/bin/sh' > "$WORK/cmdinfo/files/bin/mycommand"
chmod +x "$WORK/cmdinfo/files/bin/mycommand"
$FLATPAK build-finish "$WORK/cmdinfo" --command mycommand
$FLATPAK --user install "$WORK/cmdinfo"

output=$($FLATPAK --user info org.test.CmdInfo 2>&1)

if ! echo "$output" | grep -q "mycommand"; then
  echo "FAIL: info output does not contain 'mycommand'"
  echo "$output"
  exit 1
fi

echo "PASS: info-command"