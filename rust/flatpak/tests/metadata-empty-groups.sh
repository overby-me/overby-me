#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/emptygrp" org.test.EmptyGrp org.test.Sdk org.test.Platform

# Add an empty Context group
echo -e "\n[Context]\n" >> "$WORK/emptygrp/metadata"

# build-finish should still work with the empty group present
$FLATPAK build-finish "$WORK/emptygrp" --command test --socket x11

if ! grep -q "x11" "$WORK/emptygrp/metadata"; then
  echo "FAIL: metadata does not contain 'x11' after build-finish with empty group"
  cat "$WORK/emptygrp/metadata"
  exit 1
fi

echo "PASS: metadata-empty-groups"