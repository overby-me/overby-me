#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/extapp" org.test.Ext org.test.Sdk org.test.Platform master --extension-tag org.test.Base

if [ ! -f "$WORK/extapp/metadata" ]; then
  echo "FAIL: metadata file not created"
  exit 1
fi

if ! grep -q "\[Runtime\]\|\[Application\]" "$WORK/extapp/metadata"; then
  echo "FAIL: metadata missing [Runtime] or [Application] section"
  cat "$WORK/extapp/metadata"
  exit 1
fi

if ! grep -qi "ExtensionOf\|extension" "$WORK/extapp/metadata"; then
  echo "FAIL: metadata missing extension information"
  cat "$WORK/extapp/metadata"
  exit 1
fi

echo "PASS: build-init-extension"