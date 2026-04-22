#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_http_repo

# Add the HTTP repo as a remote
$FLATPAK --user remote-add --no-gpg-verify test-remote "$REPO_URL" 2>&1

# Search for the test app by name
output=$($FLATPAK --user search Hello 2>&1)
echo "search output: $output"

if echo "$output" | grep -q "org.test.Hello"; then
  ok "search finds app by name"
else
  echo "FAIL: search did not find org.test.Hello"
  echo "Got: $output"
  exit 1
fi

# Search for something that doesn't exist
rc=0
output=$($FLATPAK --user search NonExistentApp12345 2>&1) || rc=$?
echo "search non-existent output (rc=$rc): $output"

# Should either return non-zero or produce no matching output
if [ "$rc" -ne 0 ] || ! echo "$output" | grep -q "NonExistentApp12345"; then
  ok "search for non-existent app returns no match"
else
  echo "FAIL: search unexpectedly found NonExistentApp12345"
  exit 1
fi

echo "PASS: vm-search-remote"