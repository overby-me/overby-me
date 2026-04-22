#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that info --show-commit prints a commit hash for an app installed from remote.
# Remote installs store the commit checksum in deploy_path/commit.

setup_http_repo

# Add the HTTP repo as a remote
$FLATPAK --user remote-add --no-gpg-verify test-remote "$REPO_URL" 2>&1

# Install app from remote (this stores the commit checksum)
$FLATPAK --user install test-remote org.test.Hello 2>&1
ok "app installed from remote"

# Get the commit via info --show-commit
output=$($FLATPAK --user info --show-commit org.test.Hello 2>&1)
echo "show-commit output: $output"

# The commit should be a hex string (at least 32 chars of hex)
if echo "$output" | grep -qE '^[0-9a-f]{32,}$'; then
  ok "info --show-commit prints valid commit hash"
else
  echo "FAIL: expected hex commit hash, got: $output"
  exit 1
fi

# Verify it's non-empty and not just a path basename fallback
if [ "${#output}" -ge 32 ]; then
  ok "commit hash has reasonable length (${#output} chars)"
else
  echo "FAIL: commit hash too short: $output"
  exit 1
fi

echo "PASS: vm-info-show-commit"