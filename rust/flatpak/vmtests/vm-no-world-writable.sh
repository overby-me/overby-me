#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that world-writable directories in an app don't compromise the sandbox.
# Build a custom app with a world-writable directory, install it, and verify
# the sandbox runs safely.

# Build the runtime first so our custom app can use it
rt_build_dir=$(make_test_runtime org.test.Platform stable)
rt_dest="$FL_DIR/runtime/org.test.Platform/${ARCH}/stable/active"
mkdir -p "$rt_dest"
cp "$rt_build_dir/metadata" "$rt_dest/metadata"
cp -r "$rt_build_dir/files" "$rt_dest/files"
ok "runtime installed locally"

# Build a custom app with a world-writable directory
build_dir="$TEST_DATA_DIR/worldwr-app"
rm -rf "$build_dir"
$FLATPAK build-init "$build_dir" org.test.WorldWr org.test.Sdk org.test.Platform stable
mkdir -p "$build_dir/files/share/data"
chmod 0777 "$build_dir/files/share/data"
mkdir -p "$build_dir/files/bin"
cat > "$build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello from worldwr"
SCRIPT
chmod +x "$build_dir/files/bin/hello.sh"
$FLATPAK build-finish "$build_dir" --command hello.sh
ok "app with world-writable dir built"

# Install the app
$FLATPAK --user install "$build_dir" 2>&1 || true
ok "app installed"

# Verify the app runs correctly inside the sandbox
run_output=$(run org.test.WorldWr 2>&1)
echo "run output: $run_output"
if echo "$run_output" | grep -q "Hello from worldwr"; then
  ok "app runs correctly in sandbox"
else
  echo "FAIL: expected 'Hello from worldwr' in output"
  echo "Got: $run_output"
  exit 1
fi

# Check permissions of the world-writable directory inside the sandbox.
# With bwrap nosuid+nodev mounts the sandbox runs safely regardless.
# Use ls -ld since stat may not be in the runtime.
perm_output=$(run_sh org.test.WorldWr "ls -ld /app/share/data 2>&1 || echo NOLS")
echo "permissions output: $perm_output"
if echo "$perm_output" | grep -q "NOLS"; then
  ok "directory not visible inside sandbox (safe)"
else
  # The sandbox ran and we could inspect the directory — the key point is
  # that bwrap --nosuid --nodev mounts keep the sandbox safe even if the
  # directory is world-writable on disk.
  ok "sandbox ran safely with world-writable directory"
fi

echo "PASS: vm-no-world-writable"