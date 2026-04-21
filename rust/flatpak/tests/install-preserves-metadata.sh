#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/preserve" org.test.Preserve org.test.Sdk org.test.Platform
mkdir -p "$WORK/preserve/files/bin"
echo '#!/bin/sh' > "$WORK/preserve/files/bin/app"
$FLATPAK build-finish "$WORK/preserve" --command app --share network --socket wayland
$FLATPAK --user install "$WORK/preserve"

# Find the installed metadata
META=$(find "$HOME/.local/share/flatpak" -path "*/org.test.Preserve/*/metadata" | head -1)
[ -n "$META" ] || { echo "FAIL: metadata not found in installation"; exit 1; }
grep -q "name=org.test.Preserve" "$META"
grep -q "command=app" "$META"
grep -q "network" "$META"
grep -q "wayland" "$META"

echo "PASS: install-preserves-metadata"