#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_http_repo

# Add the HTTP repo as a remote
$FLATPAK --user remote-add --no-gpg-verify test-remote "$REPO_URL" 2>&1

# remote-ls should list app refs by default
output=$($FLATPAK --user remote-ls test-remote 2>&1)
echo "remote-ls output: $output"

if echo "$output" | grep -q "org.test.Hello"; then
  ok "remote-ls lists app ref"
else
  echo "FAIL: remote-ls did not list org.test.Hello"
  echo "Got: $output"
  exit 1
fi

# remote-ls -a should also list runtime refs
output_all=$($FLATPAK --user remote-ls -a test-remote 2>&1)
echo "remote-ls -a output: $output_all"

if echo "$output_all" | grep -q "org.test.Platform"; then
  ok "remote-ls -a lists runtime ref"
else
  echo "FAIL: remote-ls -a did not list org.test.Platform"
  echo "Got: $output_all"
  exit 1
fi

echo "PASS: vm-remote-ls"