#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build a test app
build_dir=$(make_test_app org.test.Hello stable)

# Export to an OSTree repo
$FLATPAK build-export "$TEST_DATA_DIR/bundle-repo" "$build_dir" -b stable 2>&1

# Create a .flatpak bundle from the repo
$FLATPAK build-bundle "$TEST_DATA_DIR/bundle-repo" "$TEST_DATA_DIR/hello.flatpak" "app/org.test.Hello/$ARCH/stable" 2>&1

# Verify the bundle file exists and is non-empty
assert_has_file "$TEST_DATA_DIR/hello.flatpak"
[ -s "$TEST_DATA_DIR/hello.flatpak" ] || { echo "FAIL: hello.flatpak is empty"; exit 1; }

ok "bundle created"
echo "PASS: vm-bundle-create"