#!/bin/bash
set -euo pipefail

# Build an app with two subdirectories under files/.
$FLATPAK build-init "$WORK/subapp" org.test.Sub org.test.Sdk org.test.Platform
mkdir -p "$WORK/subapp/files/bin" "$WORK/subapp/files/share"
echo '#!/bin/sh' > "$WORK/subapp/files/bin/hello"
chmod +x "$WORK/subapp/files/bin/hello"
echo "shared data" > "$WORK/subapp/files/share/data.txt"
$FLATPAK build-finish "$WORK/subapp" --command hello

# Install with --subpath=/bin only.
$FLATPAK --user install --subpath=/bin "$WORK/subapp"

INSTALL_DIR="$HOME/.local/share/flatpak"
deploy_dir=$(find "$INSTALL_DIR" -type d -name "org.test.Sub" 2>/dev/null | head -1)
if [ -z "$deploy_dir" ]; then
  echo "FAIL: org.test.Sub not installed"
  exit 1
fi

# Find the active deploy directory under that.
active=$(find "$deploy_dir" -maxdepth 4 -name "files" -type d | head -1)
if [ -z "$active" ]; then
  echo "FAIL: no files/ directory under $deploy_dir"
  exit 1
fi

if [ ! -f "$active/bin/hello" ]; then
  echo "FAIL: expected $active/bin/hello to exist"
  find "$active" -type f
  exit 1
fi

if [ -f "$active/share/data.txt" ]; then
  echo "FAIL: $active/share/data.txt should NOT exist (was excluded by --subpath)"
  exit 1
fi

# Subpaths file should be recorded next to the files directory.
parent="$(dirname "$active")"
if [ ! -f "$parent/subpaths" ]; then
  echo "FAIL: expected $parent/subpaths file"
  exit 1
fi

echo "PASS: install-subpath"
