#!/bin/bash
set -euo pipefail

rc=0
output=$("$FLATPAK" build 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: expected non-zero exit"; exit 1; }
echo "$output" | grep -qi "usage" || { echo "FAIL: expected usage hint in output"; exit 1; }

echo "PASS: build-missing-args"