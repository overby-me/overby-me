#!/bin/bash
set -euo pipefail

# Verify --verbose / -v are accepted (don't cause errors)
rc=0
$FLATPAK --verbose --user config > /dev/null 2>&1 || rc=$?
if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ]; then
  echo "FAIL: --verbose crashed"
  exit 1
fi

rc=0
$FLATPAK -v --user config > /dev/null 2>&1 || rc=$?
if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ]; then
  echo "FAIL: -v crashed"
  exit 1
fi

echo "PASS: global-verbose-flag"