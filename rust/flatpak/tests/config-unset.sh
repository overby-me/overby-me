#!/bin/bash
set -euo pipefail

# Test config --unset removes a key
$FLATPAK --user config --set languages "en;de"
output=$($FLATPAK --user config --get languages 2>&1)
[ "$output" = "en;de" ] || { echo "FAIL: expected 'en;de', got '$output'"; exit 1; }

$FLATPAK --user config --unset languages
rc=0
$FLATPAK --user config --get languages 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: config --get should fail after --unset"; exit 1; }

# Unset of non-existent key should not fail
$FLATPAK --user config --unset nonexistent-key

echo "PASS: config-unset"