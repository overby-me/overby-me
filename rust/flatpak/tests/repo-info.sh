#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/repoapp" org.test.Repo org.test.Sdk org.test.Platform
mkdir -p "$WORK/repoapp/files/bin"
echo '#!/bin/sh' > "$WORK/repoapp/files/bin/app"
chmod +x "$WORK/repoapp/files/bin/app"
$FLATPAK build-finish "$WORK/repoapp" --command app
$FLATPAK build-export "$WORK/inforepo" "$WORK/repoapp"

output=$($FLATPAK repo "$WORK/inforepo" 2>&1 || true)

# Check it doesn't crash (segfault=139, abort=134, illegal=136)
rc=0
$FLATPAK repo "$WORK/inforepo" >/dev/null 2>&1 || rc=$?
if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
  echo "FAIL: repo command crashed with signal (exit code $rc)"
  exit 1
fi

echo "PASS: repo-info"