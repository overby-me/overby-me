#!/bin/bash
set -euo pipefail

# App 1
$FLATPAK build-init "$WORK/app1" org.test.App1 org.test.Sdk org.test.Platform
mkdir -p "$WORK/app1/files/bin"
echo '#!/bin/sh' > "$WORK/app1/files/bin/app1"
chmod +x "$WORK/app1/files/bin/app1"
$FLATPAK build-finish "$WORK/app1" --command app1

# App 2
$FLATPAK build-init "$WORK/app2" org.test.App2 org.test.Sdk org.test.Platform
mkdir -p "$WORK/app2/files/bin"
echo '#!/bin/sh' > "$WORK/app2/files/bin/app2"
chmod +x "$WORK/app2/files/bin/app2"
$FLATPAK build-finish "$WORK/app2" --command app2

# Export both to same repo
$FLATPAK build-export "$WORK/multirepo" "$WORK/app1" 2>&1 || true
$FLATPAK build-export "$WORK/multirepo" "$WORK/app2" 2>&1 || true

# Both should exist in the repo
if [ ! -d "$WORK/multirepo/app/org.test.App1" ]; then
  echo "FAIL: App1 not in repo"
  ls -R "$WORK/multirepo" 2>/dev/null || true
  exit 1
fi

if [ ! -d "$WORK/multirepo/app/org.test.App2" ]; then
  echo "FAIL: App2 not in repo"
  ls -R "$WORK/multirepo" 2>/dev/null || true
  exit 1
fi

echo "PASS: build-export-multiple-apps"