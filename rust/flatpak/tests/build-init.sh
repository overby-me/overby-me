#!/bin/bash
set -euo pipefail

# Test: flatpak build-init creates app directory with metadata

$FLATPAK build-init "$WORK/testapp" org.test.App org.test.Sdk org.test.Platform master

# Check metadata file exists
if [ ! -f "$WORK/testapp/metadata" ]; then
  echo "FAIL: $WORK/testapp/metadata does not exist"
  exit 1
fi

# Check metadata contains the app name
if ! grep -q "org.test.App" "$WORK/testapp/metadata"; then
  echo "FAIL: metadata does not contain org.test.App"
  cat "$WORK/testapp/metadata"
  exit 1
fi

# Check files/ directory exists
if [ ! -d "$WORK/testapp/files" ]; then
  echo "FAIL: $WORK/testapp/files/ directory does not exist"
  exit 1
fi

echo "PASS: build-init"