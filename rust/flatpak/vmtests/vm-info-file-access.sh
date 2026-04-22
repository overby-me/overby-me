#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that info --file-access reports correct access levels based on app metadata.
# The test app has filesystems=home; in its [Context], so home should be read-write
# and paths not granted (like /usr) should be hidden.

setup_repo

# Check that home access is reported as read-write
output=$($FLATPAK --user info --file-access=home org.test.Hello 2>&1)
echo "file-access=home output: $output"
if echo "$output" | grep -qi "read-write"; then
  ok "home reported as read-write"
else
  echo "FAIL: expected 'read-write' for home access, got: $output"
  exit 1
fi

# Check that a path not granted is reported as hidden
output=$($FLATPAK --user info --file-access=/usr org.test.Hello 2>&1)
echo "file-access=/usr output: $output"
if echo "$output" | grep -qi "hidden"; then
  ok "/usr reported as hidden"
else
  echo "FAIL: expected 'hidden' for /usr access, got: $output"
  exit 1
fi

echo "PASS: vm-info-file-access"