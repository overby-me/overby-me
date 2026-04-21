#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/multigrp" org.test.MultiGrp org.test.Sdk org.test.Platform
$FLATPAK build-finish "$WORK/multigrp" --command app --share network --socket x11 --device dri --filesystem home
# Check metadata has both [Application] and [Context] groups
grep -q "^\[Application\]" "$WORK/multigrp/metadata"
grep -q "^\[Context\]" "$WORK/multigrp/metadata"
grep -q "name=org.test.MultiGrp" "$WORK/multigrp/metadata"
grep -q "command=app" "$WORK/multigrp/metadata"

echo "PASS: metadata-multiline-groups"