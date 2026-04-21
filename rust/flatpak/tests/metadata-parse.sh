#!/bin/bash
set -euo pipefail

# Test: build-init creates valid metadata that can be verified
"$FLATPAK" build-init "$WORK/testapp" org.test.App org.test.Sdk org.test.Platform

metadata="$WORK/testapp/metadata"

if [ ! -f "$metadata" ]; then
  echo "FAIL: metadata file not created at $metadata"
  exit 1
fi

grep -q "org.test.App" "$metadata" || {
  echo "FAIL: metadata does not contain 'org.test.App'"
  cat "$metadata"
  exit 1
}

grep -q "org.test.Sdk" "$metadata" || {
  echo "FAIL: metadata does not contain 'org.test.Sdk'"
  cat "$metadata"
  exit 1
}

grep -q "org.test.Platform" "$metadata" || {
  echo "FAIL: metadata does not contain 'org.test.Platform'"
  cat "$metadata"
  exit 1
}

echo "PASS: build-init creates valid metadata with expected content"