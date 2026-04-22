#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that info --show-extensions reports no extensions for an app without extension points.

setup_repo

output=$($FLATPAK --user info --show-extensions org.test.Hello 2>&1)
echo "show-extensions output: $output"

if echo "$output" | grep -qi "No extensions"; then
  ok "no extensions reported for app without extension points"
else
  echo "FAIL: expected 'No extensions' in output"
  echo "Got: $output"
  exit 1
fi

echo "PASS: vm-info-show-extensions"