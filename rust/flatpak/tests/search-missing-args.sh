#!/bin/bash
set -euo pipefail

rc=0
output=$("$FLATPAK" search 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: expected non-zero exit"; exit 1; }

echo "PASS: search-missing-args"