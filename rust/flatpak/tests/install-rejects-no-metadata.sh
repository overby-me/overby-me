#!/bin/bash
set -euo pipefail

# Build a directory with files/ but strip metadata (simulates an OSTree commit
# that is missing the xa.metadata field).
$FLATPAK build-init "$WORK/nometa" org.test.NoMeta org.test.Sdk org.test.Platform
mkdir -p "$WORK/nometa/files/bin"
echo '#!/bin/sh' > "$WORK/nometa/files/bin/h"
chmod +x "$WORK/nometa/files/bin/h"
$FLATPAK build-finish "$WORK/nometa" --command h
rm "$WORK/nometa/metadata"

set +e
out=$($FLATPAK --user install "$WORK/nometa" 2>&1)
rc=$?
set -e

if [ "$rc" -eq 0 ]; then
  echo "FAIL: install should have failed without metadata"
  echo "out: $out"
  exit 1
fi
echo "$out" | grep -q "no metadata" || { echo "FAIL: missing 'no metadata' message"; echo "$out"; exit 1; }

# Verify nothing was deployed
if find "$HOME/.local/share/flatpak" -path "*org.test.NoMeta*" -name files -type d 2>/dev/null | grep -q .; then
  echo "FAIL: app should not have been deployed"
  exit 1
fi

echo "PASS: install-rejects-no-metadata"
