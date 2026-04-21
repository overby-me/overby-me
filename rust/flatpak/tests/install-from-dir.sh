#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/installapp" org.test.Install org.test.Sdk org.test.Platform
mkdir -p "$WORK/installapp/files/bin"
echo '#!/bin/sh' > "$WORK/installapp/files/bin/hello"
chmod +x "$WORK/installapp/files/bin/hello"
$FLATPAK build-finish "$WORK/installapp" --command hello
$FLATPAK --user install "$WORK/installapp"

INSTALL_DIR="$HOME/.local/share/flatpak"
if ! find "$INSTALL_DIR" -type d -name "org.test.Install" 2>/dev/null | grep -q .; then
  echo "FAIL: org.test.Install not found in user installation directory"
  find "$INSTALL_DIR" -type d 2>/dev/null || true
  exit 1
fi

echo "PASS: install-from-dir"