#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build a test runtime
rt_build_dir=$(make_test_runtime org.test.Platform stable)

# Export to an OSTree repo
$FLATPAK build-export "$TEST_DATA_DIR/rt-repo" "$rt_build_dir" -b stable 2>&1

# Create a bundle file from the repo
$FLATPAK build-bundle "$TEST_DATA_DIR/rt-repo" "$TEST_DATA_DIR/platform.flatpak" \
  runtime/org.test.Platform/$ARCH/stable 2>&1

# Assert the bundle file exists and is non-empty
assert_has_file "$TEST_DATA_DIR/platform.flatpak"
[ -s "$TEST_DATA_DIR/platform.flatpak" ] || {
  echo "FAIL: platform.flatpak is empty"
  exit 1
}
ok "runtime bundle created"

# Import the bundle
$FLATPAK --user build-import-bundle "$TEST_DATA_DIR/platform.flatpak" 2>&1

# Verify the runtime is installed
if $FLATPAK --user list --runtime 2>&1 | grep -q "org.test.Platform"; then
  ok "runtime listed after import"
else
  # Fall back to checking the directory directly
  assert_has_file "$FL_DIR/runtime/org.test.Platform/$ARCH/stable/active/metadata"
  ok "runtime directory exists after import"
fi

echo "PASS: vm-bundle-runtime"