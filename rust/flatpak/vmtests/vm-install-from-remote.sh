#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_http_repo

# Add the HTTP repo as a remote
$FLATPAK --user remote-add --no-gpg-verify test-remote "$REPO_URL" 2>&1

# Remove locally installed app (setup_http_repo only installs runtime locally)
$FLATPAK --user uninstall org.test.Hello 2>&1 || true

# Verify app is not installed
if $FLATPAK --user list 2>&1 | grep -q "org.test.Hello"; then
  echo "FAIL: org.test.Hello should not be installed yet"
  exit 1
fi

# Install app from the HTTP remote
output=$($FLATPAK --user install test-remote org.test.Hello 2>&1)
echo "install output: $output"

# Verify app is now installed
list_output=$($FLATPAK --user list 2>&1)
echo "list output: $list_output"

if echo "$list_output" | grep -q "org.test.Hello"; then
  ok "app installed from remote"
else
  echo "FAIL: org.test.Hello not found after install"
  echo "Got: $list_output"
  exit 1
fi

# Verify the installed app can actually run
run_output=$(run org.test.Hello 2>&1)
echo "run output: $run_output"

if echo "$run_output" | grep -q "Hello world, from a sandbox"; then
  ok "app installed from remote runs correctly"
else
  echo "FAIL: expected 'Hello world, from a sandbox' in output"
  echo "Got: $run_output"
  exit 1
fi

echo "PASS: vm-install-from-remote"