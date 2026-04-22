#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test --require-version: app requiring a newer flatpak should fail to install,
# while an app requiring an older flatpak version should install fine.

# App A: requires unrealistically high version
build_dir_a="$TEST_DATA_DIR/req-app-a"
rm -rf "$build_dir_a"
$FLATPAK build-init "$build_dir_a" org.test.NeedsNewer org.test.Sdk org.test.Platform stable
mkdir -p "$build_dir_a/files/bin"
cat > "$build_dir_a/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello from needs-newer"
SCRIPT
chmod +x "$build_dir_a/files/bin/hello.sh"
$FLATPAK build-finish "$build_dir_a" --command hello.sh --require-version=99.0.0
ok "built app A requiring flatpak 99.0.0"

assert_file_has_content "$build_dir_a/metadata" "required-flatpak=99.0.0"

# Set up runtime so the install dependency is satisfied
make_test_runtime org.test.Platform stable
rt_dest="$FL_DIR/runtime/org.test.Platform/${ARCH}/stable/active"
mkdir -p "$rt_dest"
cp "$TEST_DATA_DIR/runtime-build-org.test.Platform/metadata" "$rt_dest/metadata"
cp -r "$TEST_DATA_DIR/runtime-build-org.test.Platform/files" "$rt_dest/files"
ok "runtime installed locally"

set +e
install_out=$($FLATPAK --user install "$build_dir_a" 2>&1)
install_status=$?
set -e
echo "install output: $install_out"
if [ "$install_status" -eq 0 ]; then
  echo "FAIL: install of app requiring flatpak 99.0.0 unexpectedly succeeded"
  exit 1
fi
if ! echo "$install_out" | grep -q "needs Flatpak"; then
  echo "FAIL: expected stderr to mention 'needs Flatpak'"
  echo "$install_out"
  exit 1
fi
ok "install of app A correctly rejected with version-check error"

# App B: requires a low version that we satisfy
build_dir_b="$TEST_DATA_DIR/req-app-b"
rm -rf "$build_dir_b"
$FLATPAK build-init "$build_dir_b" org.test.NeedsOld org.test.Sdk org.test.Platform stable
mkdir -p "$build_dir_b/files/bin"
cat > "$build_dir_b/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello from needs-old"
SCRIPT
chmod +x "$build_dir_b/files/bin/hello.sh"
$FLATPAK build-finish "$build_dir_b" --command hello.sh --require-version=0.0.1
ok "built app B requiring flatpak 0.0.1"

$FLATPAK --user install "$build_dir_b" 2>&1
ok "install of app B with require-version=0.0.1 succeeded"

echo "PASS: vm-version-check"
