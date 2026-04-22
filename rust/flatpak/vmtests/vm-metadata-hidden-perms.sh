#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# CVE-2021-43860: App with NUL-hidden permissions must be rejected.
# Build an app and inject NUL bytes into its metadata to hide permissions.

build_dir="$TEST_DATA_DIR/nul-app"
rm -rf "$build_dir"
$FLATPAK build-init "$build_dir" org.test.NulApp org.test.Sdk org.test.Platform stable
mkdir -p "$build_dir/files/bin"
echo '#!/bin/sh' > "$build_dir/files/bin/hello.sh"
chmod +x "$build_dir/files/bin/hello.sh"
$FLATPAK build-finish "$build_dir" --command hello.sh

# Inject NUL bytes into metadata to hide [Context] permissions
# Format: legitimate content + NUL + hidden permissions
printf '[Application]\nname=org.test.NulApp\nruntime=org.test.Platform/x86_64/stable\ncommand=hello.sh\n\0[Context]\nfilesystems=host;\n' > "$build_dir/metadata"

# Verify the file actually has NUL bytes
if ! grep -aPq "\x00" "$build_dir/metadata"; then
  echo "FAIL: test setup error - NUL byte not in metadata"
  exit 1
fi
ok "metadata file has NUL bytes (test setup)"

# Try to install — must fail due to NUL byte rejection
rc=0
output=$($FLATPAK --user install "$build_dir" 2>&1) || rc=$?
echo "install output (rc=$rc): $output"

if [ "$rc" -ne 0 ] && echo "$output" | grep -qiE "NUL|CVE-2021-43860|invalid|reject"; then
  ok "install rejected metadata with NUL bytes"
else
  echo "FAIL: install should have rejected NUL-byte metadata (rc=$rc)"
  echo "Got: $output"
  exit 1
fi

# Verify the app was NOT installed
if $FLATPAK --user list 2>&1 | grep -q "org.test.NulApp"; then
  echo "FAIL: app was installed despite NUL bytes"
  exit 1
fi
ok "app not present after rejected install"

echo "PASS: vm-metadata-hidden-perms"
