#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/exportapp" org.test.Export org.test.Sdk org.test.Platform
mkdir -p "$WORK/exportapp/files/bin"
echo '#!/bin/sh' > "$WORK/exportapp/files/bin/myapp"
chmod +x "$WORK/exportapp/files/bin/myapp"
$FLATPAK build-finish "$WORK/exportapp" --command myapp
$FLATPAK build-export "$WORK/repo" "$WORK/exportapp"

if [ ! -d "$WORK/repo" ]; then
  echo "FAIL: repo directory was not created"
  exit 1
fi

# Check that the repo has some content (ostree repo structure)
if [ -z "$(ls -A "$WORK/repo")" ]; then
  echo "FAIL: repo directory is empty"
  exit 1
fi

echo "PASS: build-export-creates-repo"