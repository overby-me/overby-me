#!/bin/bash
set -euo pipefail

rc=0
output=$("$FLATPAK" --user override --socket wayland 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: expected non-zero exit"; exit 1; }
echo "$output" | grep -qi "no application\|specified" || { echo "FAIL: no hint"; exit 1; }

echo "PASS: override-missing-app"