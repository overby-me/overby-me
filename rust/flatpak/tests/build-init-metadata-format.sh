#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/fmtapp" org.test.Format org.test.Sdk org.test.Platform stable

if ! grep -q "^\[Application\]" "$WORK/fmtapp/metadata"; then
  echo "FAIL: metadata missing [Application] group header"
  cat "$WORK/fmtapp/metadata"
  exit 1
fi

if ! grep -q "^name=org.test.Format" "$WORK/fmtapp/metadata"; then
  echo "FAIL: metadata missing name=org.test.Format"
  cat "$WORK/fmtapp/metadata"
  exit 1
fi

if ! grep -q "^runtime=org.test.Platform/" "$WORK/fmtapp/metadata"; then
  echo "FAIL: metadata missing runtime=org.test.Platform/"
  cat "$WORK/fmtapp/metadata"
  exit 1
fi

if ! grep -q "^sdk=org.test.Sdk/" "$WORK/fmtapp/metadata"; then
  echo "FAIL: metadata missing sdk=org.test.Sdk/"
  cat "$WORK/fmtapp/metadata"
  exit 1
fi

echo "PASS: build-init-metadata-format"