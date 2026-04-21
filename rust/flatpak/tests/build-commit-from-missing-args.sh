#!/bin/bash
set -euo pipefail

rc=0
output=$("$FLATPAK" build-commit-from 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: expected non-zero exit"; exit 1; }

echo "PASS: build-commit-from-missing-args"