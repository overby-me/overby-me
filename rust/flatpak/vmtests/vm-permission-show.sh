#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Empty show returns "No permissions" message.
out=$($FLATPAK permission-show org.test.NoApp 2>&1)
if ! echo "$out" | grep -q "No permissions"; then
  echo "FAIL: expected 'No permissions' for empty app"
  echo "Got: $out"
  exit 1
fi
ok "empty show works"

$FLATPAK permission-set background mybg org.test.AppX allow 2>&1
$FLATPAK permission-set notifications mynotif org.test.AppX deny 2>&1

show=$($FLATPAK permission-show org.test.AppX 2>&1)
echo "$show"
if ! echo "$show" | grep -q "background/mybg" || ! echo "$show" | grep -q "notifications/mynotif"; then
  echo "FAIL: not all permissions visible"
  echo "Got: $show"
  exit 1
fi
ok "show lists all permissions"

# Other apps should not show these.
other=$($FLATPAK permission-show org.test.Other 2>&1)
if echo "$other" | grep -q "mybg\|mynotif"; then
  echo "FAIL: leaked permissions to other app"
  exit 1
fi
ok "permissions are scoped per app-id"

echo "PASS: vm-permission-show"
