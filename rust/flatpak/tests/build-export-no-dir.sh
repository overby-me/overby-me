#!/bin/bash
set -euo pipefail

rc=0
output=$("$FLATPAK" build-export 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: expected non-zero exit"; exit 1; }
echo "$output" | grep -qi "usage" || { echo "FAIL: no usage hint"; exit 1; }

echo "PASS: build-export-no-dir"