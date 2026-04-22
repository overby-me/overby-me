#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

output=$($FLATPAK --user info --show-runtime org.test.Hello 2>&1)
echo "show-runtime output: $output"

echo "$output" | grep -q "org.test.Platform" || {
  echo "FAIL: expected 'org.test.Platform' in show-runtime output"
  echo "Got: $output"
  exit 1
}

ok "show-runtime prints runtime ref"
echo "PASS: vm-info-show-runtime"