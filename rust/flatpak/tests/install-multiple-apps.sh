#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/multi1" org.test.Multi1 org.test.Sdk org.test.Platform
mkdir -p "$WORK/multi1/files/bin"
echo '#!/bin/sh' > "$WORK/multi1/files/bin/app"
chmod +x "$WORK/multi1/files/bin/app"
$FLATPAK build-finish "$WORK/multi1" --command app

$FLATPAK build-init "$WORK/multi2" org.test.Multi2 org.test.Sdk org.test.Platform
mkdir -p "$WORK/multi2/files/bin"
echo '#!/bin/sh' > "$WORK/multi2/files/bin/app"
chmod +x "$WORK/multi2/files/bin/app"
$FLATPAK build-finish "$WORK/multi2" --command app

$FLATPAK --user install "$WORK/multi1"
$FLATPAK --user install "$WORK/multi2"

$FLATPAK --user list > "$WORK/list_out" 2>&1

if ! grep -q "org.test.Multi1" "$WORK/list_out"; then
  echo "FAIL: org.test.Multi1 not found in list output"
  cat "$WORK/list_out"
  exit 1
fi

if ! grep -q "org.test.Multi2" "$WORK/list_out"; then
  echo "FAIL: org.test.Multi2 not found in list output"
  cat "$WORK/list_out"
  exit 1
fi

echo "PASS: install-multiple-apps"