#!/bin/bash
set -euo pipefail

# Test config --set / --get round-trip
$FLATPAK --user config --set languages "en;de"
output=$($FLATPAK --user config --get languages 2>&1)
[ "$output" = "en;de" ] || { echo "FAIL: expected 'en;de', got '$output'"; exit 1; }

# Overwrite
$FLATPAK --user config --set languages "en;fr"
output=$($FLATPAK --user config --get languages 2>&1)
[ "$output" = "en;fr" ] || { echo "FAIL: expected 'en;fr', got '$output'"; exit 1; }

echo "PASS: config-set-get"