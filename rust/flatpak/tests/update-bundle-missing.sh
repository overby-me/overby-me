#!/bin/bash
set -euo pipefail

# Verify `flatpak update --bundle=PATH` rejects nonexistent bundles.
output=$($FLATPAK --user update --bundle=/nonexistent.flatpak 2>&1 || true)
if ! echo "$output" | grep -q "bundle not found"; then
  echo "FAIL: expected 'bundle not found' in stderr"
  echo "Got: $output"
  exit 1
fi
echo "PASS: update-bundle-missing"
