#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that build-finish --sdk= is recorded in metadata

build_dir="$TEST_DATA_DIR/sdk-test-app"
rm -rf "$build_dir"
$FLATPAK build-init "$build_dir" org.test.SdkApp org.test.Sdk org.test.Platform stable

mkdir -p "$build_dir/files/bin"
cat > "$build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello from sdk test"
SCRIPT
chmod +x "$build_dir/files/bin/hello.sh"

# Use --sdk= option
$FLATPAK build-finish "$build_dir" --command hello.sh --sdk=org.test.Sdk/x86_64/stable

# Verify sdk is recorded in the metadata file
assert_has_file "$build_dir/metadata"
assert_file_has_content "$build_dir/metadata" "sdk=org.test.Sdk/x86_64/stable"
ok "build-finish --sdk= recorded in metadata"

# Also test the --sdk KEY form (space-separated)
build_dir2="$TEST_DATA_DIR/sdk-test-app2"
rm -rf "$build_dir2"
$FLATPAK build-init "$build_dir2" org.test.SdkApp2 org.test.Sdk org.test.Platform stable

mkdir -p "$build_dir2/files/bin"
cat > "$build_dir2/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello from sdk test 2"
SCRIPT
chmod +x "$build_dir2/files/bin/hello.sh"

$FLATPAK build-finish "$build_dir2" --command hello.sh --sdk org.test.Sdk/x86_64/stable

assert_has_file "$build_dir2/metadata"
assert_file_has_content "$build_dir2/metadata" "sdk=org.test.Sdk/x86_64/stable"
ok "build-finish --sdk (space-separated) recorded in metadata"

echo "PASS: vm-sdk-option"