#!/bin/bash
set -euo pipefail

$FLATPAK -u config > "$WORK/u_out"
$FLATPAK --user config > "$WORK/user_out"
# Both should produce output containing "user"
grep -q "user" "$WORK/u_out"
grep -q "user" "$WORK/user_out"

echo "PASS: global-user-flag"