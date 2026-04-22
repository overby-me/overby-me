#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build v1 of the app and export to repo
build_dir=$(make_test_app org.test.Hello stable)
$FLATPAK build-export "$TEST_DATA_DIR/bundle-repo" "$build_dir" -b stable 2>&1

# Create v1 bundle
$FLATPAK build-bundle "$TEST_DATA_DIR/bundle-repo" "$TEST_DATA_DIR/hello-v1.flatpak" "app/org.test.Hello/$ARCH/stable" 2>&1
ok "v1 bundle created"

# Install runtime locally so the app can run
rt_build_dir=$(make_test_runtime org.test.Platform stable)
rt_dest="$FL_DIR/runtime/org.test.Platform/${ARCH}/stable/active"
mkdir -p "$rt_dest"
cp "$rt_build_dir/metadata" "$rt_dest/metadata"
cp -r "$rt_build_dir/files" "$rt_dest/files"
ok "runtime installed locally"

# Import v1 bundle (initial install)
$FLATPAK --user build-import-bundle "$TEST_DATA_DIR/hello-v1.flatpak" 2>&1
ok "v1 bundle imported"

# Verify v1 runs
run_output=$(run org.test.Hello 2>&1)
if echo "$run_output" | grep -q "Hello world, from a sandbox"; then
  ok "v1 runs correctly"
else
  echo "FAIL: expected 'Hello world, from a sandbox' in output"
  echo "Got: $run_output"
  exit 1
fi

# Modify the app to produce v2 output
cat > "$build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello v2, updated via bundle"
SCRIPT
chmod +x "$build_dir/files/bin/hello.sh"

# Re-export v2 and create v2 bundle
$FLATPAK build-export "$TEST_DATA_DIR/bundle-repo" "$build_dir" -b stable 2>&1
$FLATPAK build-bundle "$TEST_DATA_DIR/bundle-repo" "$TEST_DATA_DIR/hello-v2.flatpak" "app/org.test.Hello/$ARCH/stable" 2>&1
ok "v2 bundle created"

# Use `flatpak update --bundle=PATH` to refresh the installed app
update_output=$($FLATPAK --user update --bundle="$TEST_DATA_DIR/hello-v2.flatpak" 2>&1)
echo "update output: $update_output"
if echo "$update_output" | grep -q "Updated from bundle"; then
  ok "update --bundle reported success"
else
  echo "FAIL: expected 'Updated from bundle' message"
  echo "Got: $update_output"
  exit 1
fi

# Verify v2 runs correctly
run_output=$(run org.test.Hello 2>&1)
if echo "$run_output" | grep -q "Hello v2, updated via bundle"; then
  ok "v2 runs correctly after bundle update"
else
  echo "FAIL: expected 'Hello v2, updated via bundle' in output"
  echo "Got: $run_output"
  exit 1
fi

echo "PASS: vm-bundle-update"
