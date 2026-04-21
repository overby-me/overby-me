#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

setup_repo
output=$(run --command sh org.test.Hello -c 'echo custom-command' 2>&1)
echo "$output" | grep -q "custom-command" || {
  echo "FAIL: --command override didn't work"
  echo "Got: $output"
  exit 1
}
ok "run command override"

echo "PASS: vm-run-command-override"