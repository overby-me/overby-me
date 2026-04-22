#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build v1 of the app and export to repo
build_dir=$(make_test_app org.test.Hello stable)
$FLATPAK build-export "$TEST_DATA_DIR/bundle-repo" "$build_dir" -b stable 2>&1

# Create v1 bundle
$FLATPAK build-bundle "$TEST_DATA_DIR/bundle-repo" "$TEST_DATA_DIR/hello-v1.flatpak" "app/org.test.Hello/$ARCH/stable" 2>&1
assert_has_file "$TEST_DATA_DIR/hello-v1.flatpak"
ok "v1 bundle created"

# Install runtime locally so the app can run
rt_build_dir=$(make_test_runtime org.test.Platform stable)
rt_dest="$FL_DIR/runtime/org.test.Platform/${ARCH}/stable/active"
mkdir -p "$rt_dest"
cp "$rt_build_dir/metadata" "$rt_dest/metadata"
cp -r "$rt_build_dir/files" "$rt_dest/files"
ok "runtime installed locally"

# Import v1 bundle
$FLATPAK --user build-import-bundle "$TEST_DATA_DIR/hello-v1.flatpak" 2>&1
ok "v1 bundle imported"

# Verify v1 runs
run_output=$(run org.test.Hello 2>&1)
echo "v1 run output: $run_output"
if echo "$run_output" | grep -q "Hello world, from a sandbox"; then
  ok "v1 app runs correctly"
else
  echo "FAIL: expected 'Hello world, from a sandbox' in output"
  echo "Got: $run_output"
  exit 1
fi

# Modify app to produce v2 output
cat > "$build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello v2, updated"
SCRIPT
chmod +x "$build_dir/files/bin/hello.sh"

# Re-export v2 to the same repo
$FLATPAK build-export "$TEST_DATA_DIR/bundle-repo" "$build_dir" -b stable 2>&1
$FLATPAK build-update-repo "$TEST_DATA_DIR/bundle-repo" 2>&1
ok "v2 exported to repo"

# Create v2 bundle
$FLATPAK build-bundle "$TEST_DATA_DIR/bundle-repo" "$TEST_DATA_DIR/hello-v2.flatpak" "app/org.test.Hello/$ARCH/stable" 2>&1
assert_has_file "$TEST_DATA_DIR/hello-v2.flatpak"
ok "v2 bundle created"

# Import v2 bundle (updates the existing app)
$FLATPAK --user build-import-bundle "$TEST_DATA_DIR/hello-v2.flatpak" 2>&1
ok "v2 bundle imported"

# Verify v2 runs
run_output=$(run org.test.Hello 2>&1)
echo "v2 run output: $run_output"
if echo "$run_output" | grep -q "Hello v2, updated"; then
  ok "v2 app runs correctly after bundle update"
else
  echo "FAIL: expected 'Hello v2, updated' in output"
  echo "Got: $run_output"
  exit 1
fi

echo "PASS: vm-bundle-update-as-bundle"