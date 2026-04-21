#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/myapp" org.test.App org.test.Sdk org.test.Platform
$FLATPAK build-finish "$WORK/myapp" --command mycommand

if [ ! -f "$WORK/myapp/metadata" ]; then
  echo "FAIL: metadata file not found"
  exit 1
fi

if ! grep -q "command=mycommand" "$WORK/myapp/metadata"; then
  echo "FAIL: metadata does not contain 'command=mycommand'"
  cat "$WORK/myapp/metadata"
  exit 1
fi

echo "PASS: build-finish-command"