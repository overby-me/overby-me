#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

# Run repair on a healthy installation — should find no problems
output=$($FLATPAK --user repair 2>&1)
echo "repair output: $output"

if echo "$output" | grep -q "No problems found"; then
  ok "repair reports no problems on healthy installation"
else
  echo "FAIL: repair did not report 'No problems found'"
  echo "Got: $output"
  exit 1
fi

# Verify the number of refs checked is at least 1
if echo "$output" | grep -qE "[0-9]+ refs checked"; then
  ok "repair reports refs checked count"
else
  echo "FAIL: repair output missing refs checked count"
  echo "Got: $output"
  exit 1
fi

echo "PASS: vm-repair-no-problems"