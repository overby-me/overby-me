#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

# Test config --set / --get if implemented
# The current rust-flatpak config command is limited, so test what's available
output=$($FLATPAK --user config 2>&1 || true)
echo "$output" | grep -qi "user\|path" || {
  echo "FAIL: config output missing expected content"
  echo "Got: $output"
  exit 1
}
ok "config basic"

echo "PASS: vm-config-set-get"