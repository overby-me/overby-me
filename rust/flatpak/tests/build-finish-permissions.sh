#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/permapp" org.test.App org.test.Sdk org.test.Platform
$FLATPAK build-finish "$WORK/permapp" --command app --share network --socket x11 --socket wayland --filesystem home --device dri

META="$WORK/permapp/metadata"

if [ ! -f "$META" ]; then
  echo "FAIL: metadata file not found at $META"
  exit 1
fi

if ! grep -q "command=app" "$META"; then
  echo "FAIL: metadata does not contain command=app"
  cat "$META"
  exit 1
fi

if ! grep -qi "shared.*network\|network.*shared" "$META" && ! grep -q "network" "$META"; then
  echo "FAIL: metadata does not contain network in shared"
  cat "$META"
  exit 1
fi

if ! grep -q "x11" "$META"; then
  echo "FAIL: metadata does not contain x11 in sockets"
  cat "$META"
  exit 1
fi

if ! grep -q "wayland" "$META"; then
  echo "FAIL: metadata does not contain wayland in sockets"
  cat "$META"
  exit 1
fi

if ! grep -q "home" "$META"; then
  echo "FAIL: metadata does not contain home in filesystems"
  cat "$META"
  exit 1
fi

if ! grep -q "dri" "$META"; then
  echo "FAIL: metadata does not contain dri in devices"
  cat "$META"
  exit 1
fi

echo "PASS: build-finish-permissions"