#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

$FLATPAK permission-set notifications gone org.test.App allow 2>&1
ok "set"
$FLATPAK permission-show org.test.App | grep -q "notifications/gone" || { echo "FAIL: setup"; exit 1; }

$FLATPAK permission-remove notifications gone 2>&1
ok "remove"

show=$($FLATPAK permission-show org.test.App 2>&1)
if echo "$show" | grep -q "notifications/gone"; then
  echo "FAIL: permission still present after remove"
  echo "Got: $show"
  exit 1
fi
ok "permission no longer present"

echo "PASS: vm-permission-remove"
