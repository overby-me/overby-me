#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

output=$($FLATPAK --user info --show-sdk org.test.Hello 2>&1)
echo "info --show-sdk output: $output"

if echo "$output" | grep -q "org.test.Sdk"; then
  ok "show-sdk contains org.test.Sdk"
else
  echo "FAIL: expected 'org.test.Sdk' in output (derived from Platform→Sdk)"
  echo "Got: $output"
  exit 1
fi

echo "PASS: vm-info-show-sdk"