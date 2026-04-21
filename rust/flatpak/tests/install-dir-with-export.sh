#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/exportinst" org.test.ExportInst org.test.Sdk org.test.Platform
mkdir -p "$WORK/exportinst/files/share/applications"
echo -e "[Desktop Entry]\nName=Test\nExec=test\nType=Application" > "$WORK/exportinst/files/share/applications/org.test.ExportInst.desktop"
mkdir -p "$WORK/exportinst/files/bin"
echo '#!/bin/sh' > "$WORK/exportinst/files/bin/test"
chmod +x "$WORK/exportinst/files/bin/test"
$FLATPAK build-finish "$WORK/exportinst" --command test

$FLATPAK --user install "$WORK/exportinst"

INST_DIR="$HOME/.local/share/flatpak"

# At minimum the metadata should be there
if ! find "$INST_DIR" -name "metadata" -exec grep -l "org.test.ExportInst" {} + > /dev/null 2>&1; then
  echo "FAIL: metadata for org.test.ExportInst not found in install dir"
  find "$INST_DIR" -type f 2>/dev/null || true
  exit 1
fi

echo "PASS: install-dir-with-export"