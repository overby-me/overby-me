#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build a test app
build_dir=$(make_test_app org.test.Title stable)

# Export to an OSTree repo
$FLATPAK build-export "$TEST_DATA_DIR/title-repo" "$build_dir" -b stable 2>&1

# Set a title on the repo
$FLATPAK build-update-repo --title="Test Repo" "$TEST_DATA_DIR/title-repo" 2>&1

# Verify the config contains the expected title
grep -q "title=Test Repo" "$TEST_DATA_DIR/title-repo/config" || {
  echo "FAIL: title not found in repo config"
  cat "$TEST_DATA_DIR/title-repo/config"
  exit 1
}

ok "title set in repo config"
echo "PASS: vm-build-update-repo-title"