#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/redir-app" org.test.Redir org.test.Sdk org.test.Platform
mkdir -p "$WORK/redir-app/files/bin"
echo '#!/bin/sh' > "$WORK/redir-app/files/bin/app"
$FLATPAK build-finish "$WORK/redir-app" --command app
$FLATPAK build-export "$WORK/redir-repo" "$WORK/redir-app" 2>&1
$FLATPAK build-update-repo --redirect-url=http://example.com/redir "$WORK/redir-repo" 2>&1

grep -q "redirect-url=http://example.com/redir" "$WORK/redir-repo/config" || { echo "FAIL: redirect-url not found in repo config"; exit 1; }

echo "PASS: build-update-repo-redirect"