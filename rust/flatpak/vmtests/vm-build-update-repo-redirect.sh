#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build a test app
build_dir=$(make_test_app org.test.Redir stable)

# Export to an OSTree repo
$FLATPAK build-export "$TEST_DATA_DIR/redir-repo" "$build_dir" -b stable 2>&1

# Set redirect-url on the repo
$FLATPAK build-update-repo --redirect-url=http://example.com/new "$TEST_DATA_DIR/redir-repo" 2>&1

# Verify the config contains the redirect-url
grep -q "redirect-url=http://example.com/new" "$TEST_DATA_DIR/redir-repo/config" || {
  echo "FAIL: redirect-url not found in repo config"
  exit 1
}

ok "redirect-url set correctly"
echo "PASS: vm-build-update-repo-redirect"