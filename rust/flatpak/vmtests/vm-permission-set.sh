#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

out=$($FLATPAK permission-set notifications myid org.test.App allow 2>&1)
echo "$out"
if ! echo "$out" | grep -qi "permission set"; then
  echo "FAIL: expected 'Permission set' message"
  exit 1
fi
ok "permission-set succeeded"

# show should now include it
show=$($FLATPAK permission-show org.test.App 2>&1)
echo "$show"
if ! echo "$show" | grep -q "notifications/myid"; then
  echo "FAIL: permission not visible via show"
  exit 1
fi
ok "permission visible via show"

echo "PASS: vm-permission-set"
