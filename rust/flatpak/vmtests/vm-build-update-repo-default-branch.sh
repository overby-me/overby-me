#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build a test app
build_dir=$(make_test_app org.test.DefBranch stable)

# Export to an OSTree repo
$FLATPAK build-export "$TEST_DATA_DIR/branch-repo" "$build_dir" -b stable 2>&1

# Set the default branch on the repo
$FLATPAK build-update-repo --default-branch=beta "$TEST_DATA_DIR/branch-repo" 2>&1

# Verify the config file contains the expected default-branch
grep -q "default-branch=beta" "$TEST_DATA_DIR/branch-repo/config" || {
  echo "FAIL: default-branch not found in repo config"
  cat "$TEST_DATA_DIR/branch-repo/config"
  exit 1
}

ok "default-branch set in repo config"
echo "PASS: vm-build-update-repo-default-branch"