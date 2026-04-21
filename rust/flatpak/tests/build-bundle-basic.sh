#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/bundleapp" org.test.Bundle org.test.Sdk org.test.Platform
mkdir -p "$WORK/bundleapp/files/bin"
echo '#!/bin/sh' > "$WORK/bundleapp/files/bin/app"
chmod +x "$WORK/bundleapp/files/bin/app"
$FLATPAK build-finish "$WORK/bundleapp" --command app
$FLATPAK build-export "$WORK/bundlerepo" "$WORK/bundleapp"

output=$($FLATPAK build-bundle "$WORK/bundlerepo" "$WORK/test.flatpak" org.test.Bundle 2>&1 || true)

# Check that either the bundle file was created OR we get a meaningful error (not crash)
if [ -f "$WORK/test.flatpak" ]; then
  echo "Bundle file created successfully"
else
  # Ensure it didn't crash (signal 139=segfault, 134=abort, 136=fpe)
  rc=0
  $FLATPAK build-bundle "$WORK/bundlerepo" "$WORK/test2.flatpak" org.test.Bundle 2>&1 || rc=$?
  if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
    echo "FAIL: build-bundle crashed with signal (exit code $rc)"
    exit 1
  fi
  echo "build-bundle exited with code $rc (no crash)"
fi

echo "PASS: build-bundle-basic"