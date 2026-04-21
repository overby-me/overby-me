#!/bin/bash
set -euo pipefail

rc=0
output=$($FLATPAK --user uninstall org.test.DoesNotExist 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: expected non-zero exit"; exit 1; }

echo "PASS: uninstall-nonexistent"