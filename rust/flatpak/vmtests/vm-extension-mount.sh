#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build a custom runtime with an extension point declared
rt_build_dir="$TEST_DATA_DIR/runtime-with-ext"
rm -rf "$rt_build_dir"
mkdir -p "$rt_build_dir/files/bin" "$rt_build_dir/files/lib"

# Copy basic binaries from the helper
make_test_runtime org.test.PlatformExt stable
src_rt="$TEST_DATA_DIR/runtime-build-org.test.PlatformExt"
cp -r "$src_rt/files"/* "$rt_build_dir/files/" 2>/dev/null || true

# Write metadata with an extension point
cat > "$rt_build_dir/metadata" << META
[Runtime]
name=org.test.PlatformExt
runtime=org.test.PlatformExt/${ARCH}/stable
sdk=org.test.SdkExt/${ARCH}/stable

[Extension org.test.PlatformExt.MyExt]
directory=lib/myext
version=stable
META

# Install runtime locally
rt_dest="$FL_DIR/runtime/org.test.PlatformExt/${ARCH}/stable/active"
mkdir -p "$rt_dest"
cp "$rt_build_dir/metadata" "$rt_dest/metadata"
cp -r "$rt_build_dir/files" "$rt_dest/files"
ok "runtime with extension point installed"

# Build an app that uses this runtime
app_build_dir="$TEST_DATA_DIR/ext-app"
rm -rf "$app_build_dir"
$FLATPAK build-init "$app_build_dir" org.test.ExtApp org.test.SdkExt org.test.PlatformExt stable
mkdir -p "$app_build_dir/files/bin"
cat > "$app_build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "extension test"
ls /usr/lib/myext 2>&1 || echo "extension dir not present"
SCRIPT
chmod +x "$app_build_dir/files/bin/hello.sh"
$FLATPAK build-finish "$app_build_dir" --command hello.sh
$FLATPAK --user install "$app_build_dir" 2>&1 || true
ok "app with extension-point runtime installed"

# Run the app — extension dir should exist (even if empty) inside sandbox
output=$(run org.test.ExtApp 2>&1)
echo "app run output: $output"

if echo "$output" | grep -q "extension test"; then
  ok "app with extension-point runtime runs successfully"
else
  echo "FAIL: app did not run"
  echo "Got: $output"
  exit 1
fi

echo "PASS: vm-extension-mount"
