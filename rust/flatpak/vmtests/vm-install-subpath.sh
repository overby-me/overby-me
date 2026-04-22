#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build a custom test app with two subdirectories under files/.
build_dir="$TEST_DATA_DIR/subpath-app"
rm -rf "$build_dir"
$FLATPAK build-init "$build_dir" org.test.Sub org.test.Sdk org.test.Platform stable
mkdir -p "$build_dir/files/bin" "$build_dir/files/share"
cat > "$build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello from subpath test"
SCRIPT
chmod +x "$build_dir/files/bin/hello.sh"
echo "shared data" > "$build_dir/files/share/data.txt"
$FLATPAK build-finish "$build_dir" --command hello.sh 2>&1
ok "built app with bin/ and share/"

# Install with --subpath=/bin only.
$FLATPAK --user install --subpath=/bin "$build_dir" 2>&1
ok "subpath install completed"

deploy="$FL_DIR/app/org.test.Sub/${ARCH}/stable/active"
assert_has_dir "$deploy/files/bin"
assert_has_file "$deploy/files/bin/hello.sh"
assert_not_has_file "$deploy/files/share/data.txt"
assert_has_file "$deploy/subpaths"
ok "only /bin was installed; share/ excluded; subpaths file written"

echo "PASS: vm-install-subpath"
