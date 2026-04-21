#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/brapp" org.test.BrApp org.test.Sdk org.test.Platform mybranch
grep -q "mybranch" "$WORK/brapp/metadata"

echo "PASS: build-init-custom-branch"