#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/myapp" org.test.App org.test.Sdk org.test.Platform
$FLATPAK build-finish "$WORK/myapp" --command foo --require-version=0.0.1

if ! grep -q "required-flatpak=0.0.1" "$WORK/myapp/metadata"; then
  echo "FAIL: metadata does not contain 'required-flatpak=0.0.1'"
  cat "$WORK/myapp/metadata"
  exit 1
fi

echo "PASS: build-finish-require-version"
