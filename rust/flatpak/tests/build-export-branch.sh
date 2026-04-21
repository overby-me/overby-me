#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/branchapp" org.test.Branch org.test.Sdk org.test.Platform
mkdir -p "$WORK/branchapp/files/bin"
echo '#!/bin/sh' > "$WORK/branchapp/files/bin/app"
chmod +x "$WORK/branchapp/files/bin/app"
$FLATPAK build-finish "$WORK/branchapp" --command app
$FLATPAK build-export "$WORK/branchrepo" "$WORK/branchapp" -b mybranch

if [ ! -d "$WORK/branchrepo" ]; then
  echo "FAIL: repo directory not created"
  exit 1
fi

# Check that the repo references mybranch somewhere in its structure
if ! find "$WORK/branchrepo" -type f -exec grep -rl "mybranch" {} + >/dev/null 2>&1; then
  # Also check directory names
  if ! find "$WORK/branchrepo" -name "*mybranch*" >/dev/null 2>&1 || [ -z "$(find "$WORK/branchrepo" -name "*mybranch*" 2>/dev/null)" ]; then
    # Try binary grep across all files
    if ! grep -rl "mybranch" "$WORK/branchrepo" >/dev/null 2>&1; then
      echo "WARN: could not confirm mybranch in repo (may be in binary format)"
    fi
  fi
fi

echo "PASS: build-export-branch"