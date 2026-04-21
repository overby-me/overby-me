#!/bin/bash
set -euo pipefail

# Build
$FLATPAK build-init "$WORK/fullapp" org.test.Full org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/fullapp/files/bin"
echo '#!/bin/sh' > "$WORK/fullapp/files/bin/myapp"
chmod +x "$WORK/fullapp/files/bin/myapp"
$FLATPAK build-finish "$WORK/fullapp" --command myapp --share network --socket x11

# Install
$FLATPAK --user install "$WORK/fullapp"

# List shows it
$FLATPAK --user list > "$WORK/list_out"
grep -q "org.test.Full" "$WORK/list_out"

# Info works
$FLATPAK --user info org.test.Full > "$WORK/info_out"
grep -q "org.test.Full" "$WORK/info_out"

# Info with metadata
$FLATPAK --user info --show-metadata org.test.Full > "$WORK/meta_out"
grep -q "name=org.test.Full" "$WORK/meta_out"
grep -q "command=myapp" "$WORK/meta_out"

# Uninstall
$FLATPAK --user uninstall org.test.Full

# List no longer shows it
$FLATPAK --user list > "$WORK/list_out2"
if grep -q "org.test.Full" "$WORK/list_out2"; then
  echo "FAIL: app still in list after uninstall"
  exit 1
fi

echo "PASS: install-then-list"