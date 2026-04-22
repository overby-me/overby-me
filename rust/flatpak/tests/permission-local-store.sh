#!/bin/bash
set -euo pipefail

# Local fallback portal: works without a real session bus.
export DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent

$FLATPAK permission-set notifications myid org.test.App allow
$FLATPAK permission-set background bgid org.test.App deny

show=$($FLATPAK permission-show org.test.App 2>&1)
if ! echo "$show" | grep -q "notifications/myid"; then
  echo "FAIL: notifications/myid missing"; echo "$show"; exit 1
fi
if ! echo "$show" | grep -q "background/bgid"; then
  echo "FAIL: background/bgid missing"; echo "$show"; exit 1
fi

# Other apps don't see these.
other=$($FLATPAK permission-show org.test.NeverSeenBefore.X9XQ 2>&1)
echo "$other" | grep -q "No permissions" || { echo "FAIL: scoped show broken"; echo "$other"; exit 1; }

# Remove + reset.
$FLATPAK permission-remove notifications myid
remaining=$($FLATPAK permission-show org.test.App 2>&1)
echo "$remaining" | grep -q "notifications/myid" && { echo "FAIL: not removed"; exit 1; } || true

$FLATPAK permission-reset org.test.App
final=$($FLATPAK permission-show org.test.App 2>&1)
echo "$final" | grep -q "No permissions" || { echo "FAIL: not reset"; echo "$final"; exit 1; }

echo "PASS: permission-local-store"
