#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

output=$(run org.test.Hello 2>&1)
echo "$output" | grep -q "Hello world, from a sandbox" || {
  echo "FAIL: expected 'Hello world, from a sandbox' in output"
  echo "Got: $output"
  exit 1
}

ok "run hello"
echo "PASS: vm-run-hello"