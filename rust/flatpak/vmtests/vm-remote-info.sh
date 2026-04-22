#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_http_repo

# Add the HTTP repo as a remote
$FLATPAK --user remote-add --no-gpg-verify test-remote "$REPO_URL" 2>&1

# remote-info should show details for a specific ref
output=$($FLATPAK --user remote-info test-remote org.test.Hello 2>&1)
echo "remote-info output: $output"

if echo "$output" | grep -q "org.test.Hello"; then
  ok "remote-info shows app id"
else
  echo "FAIL: remote-info did not show org.test.Hello"
  echo "Got: $output"
  exit 1
fi

# Also test with runtime ref
output=$($FLATPAK --user remote-info test-remote org.test.Platform 2>&1)
echo "remote-info runtime output: $output"

if echo "$output" | grep -q "org.test.Platform"; then
  ok "remote-info shows runtime id"
else
  echo "FAIL: remote-info did not show org.test.Platform"
  echo "Got: $output"
  exit 1
fi

echo "PASS: vm-remote-info"