#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

$FLATPAK permission-set notifications a org.test.Reset allow 2>&1
$FLATPAK permission-set background b org.test.Reset allow 2>&1
$FLATPAK permission-set devices c org.test.Reset allow 2>&1
ok "seeded permissions across 3 tables"

before=$($FLATPAK permission-show org.test.Reset 2>&1 | grep -c ":" || true)
if [ "$before" -lt 3 ]; then
  echo "FAIL: expected at least 3 permission rows before reset, got $before"
  $FLATPAK permission-show org.test.Reset
  exit 1
fi

$FLATPAK permission-reset org.test.Reset 2>&1
ok "reset"

after=$($FLATPAK permission-show org.test.Reset 2>&1)
if ! echo "$after" | grep -q "No permissions"; then
  echo "FAIL: expected 'No permissions' after reset"
  echo "Got: $after"
  exit 1
fi
ok "no permissions remain after reset"

# Permissions for other apps must survive.
$FLATPAK permission-set notifications x org.test.Other allow 2>&1
$FLATPAK permission-reset org.test.Reset 2>&1
other=$($FLATPAK permission-show org.test.Other 2>&1)
if ! echo "$other" | grep -q "notifications/x"; then
  echo "FAIL: reset clobbered other app permissions"
  echo "Got: $other"
  exit 1
fi
ok "reset is scoped to target app"

echo "PASS: vm-permission-reset"
