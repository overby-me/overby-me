#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that invalid metadata syntax is rejected.

build_dir="$TEST_DATA_DIR/invalid-app"
rm -rf "$build_dir"
$FLATPAK build-init "$build_dir" org.test.InvalidApp org.test.Sdk org.test.Platform stable
mkdir -p "$build_dir/files/bin"
echo '#!/bin/sh' > "$build_dir/files/bin/hello.sh"
chmod +x "$build_dir/files/bin/hello.sh"
$FLATPAK build-finish "$build_dir" --command hello.sh

# Replace metadata with malformed content (no [Application] or [Runtime] group,
# no name= key — required fields are missing)
cat > "$build_dir/metadata" << META
this is not valid metadata at all
just garbage with = signs
random=stuff
META

# Try to install — should fail because metadata is missing required fields
rc=0
output=$($FLATPAK --user install "$build_dir" 2>&1) || rc=$?
echo "install output (rc=$rc): $output"

if [ "$rc" -ne 0 ]; then
  ok "install rejected invalid metadata"
else
  # If install succeeded, verify the app id was at least correctly parsed
  # (it shouldn't be, since there's no [Application] group with name=)
  if $FLATPAK --user list 2>&1 | grep -q "org.test.InvalidApp"; then
    echo "FAIL: invalid metadata accepted and app installed"
    exit 1
  fi
  ok "install completed but app not actually installed (acceptable)"
fi

echo "PASS: vm-metadata-invalid"
