#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build test app and export to repo
build_dir=$(make_test_app org.test.Hello stable)
$FLATPAK build-export "$TEST_DATA_DIR/bundle-repo" "$build_dir" -b stable 2>&1

# Create bundle file from repo
$FLATPAK build-bundle "$TEST_DATA_DIR/bundle-repo" "$TEST_DATA_DIR/hello.flatpak" "app/org.test.Hello/$ARCH/stable" 2>&1

assert_has_file "$TEST_DATA_DIR/hello.flatpak"
[ -s "$TEST_DATA_DIR/hello.flatpak" ] || { echo "FAIL: hello.flatpak is empty"; exit 1; }
ok "bundle created"

# Install runtime locally so the app can run
rt_build_dir=$(make_test_runtime org.test.Platform stable)
rt_dest="$FL_DIR/runtime/org.test.Platform/${ARCH}/stable/active"
mkdir -p "$rt_dest"
cp "$rt_build_dir/metadata" "$rt_dest/metadata"
cp -r "$rt_build_dir/files" "$rt_dest/files"
ok "runtime installed locally"

# Import the bundle
$FLATPAK --user build-import-bundle "$TEST_DATA_DIR/hello.flatpak" 2>&1
ok "bundle imported"

# Verify it's installed
list_output=$($FLATPAK --user list 2>&1)
if echo "$list_output" | grep -q "org.test.Hello"; then
  ok "app listed after bundle import"
else
  echo "FAIL: org.test.Hello not found in list"
  echo "Got: $list_output"
  exit 1
fi

# Verify it can run
run_output=$(run org.test.Hello 2>&1)
if echo "$run_output" | grep -q "Hello world, from a sandbox"; then
  ok "app runs correctly from bundle"
else
  echo "FAIL: expected 'Hello world, from a sandbox' in output"
  echo "Got: $run_output"
  exit 1
fi

echo "PASS: vm-bundle-install"