#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

output=$($FLATPAK --user info --show-location org.test.Hello 2>&1)
echo "show-location output: $output"

echo "$output" | grep -q "org.test.Hello" || {
  echo "FAIL: expected 'org.test.Hello' in show-location output"
  echo "Got: $output"
  exit 1
}

if [ ! -d "$output" ]; then
  echo "FAIL: show-location path does not exist as a directory"
  echo "Got: $output"
  exit 1
fi

ok "show-location prints deploy path"
echo "PASS: vm-info-show-location"