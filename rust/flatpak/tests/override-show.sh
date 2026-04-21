#!/bin/bash
set -euo pipefail

$FLATPAK --user override --socket x11 --device dri --share network --env FOO=bar --filesystem home org.test.ShowTest

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.ShowTest"
grep -q "\[Context\]" "$OVERRIDE_FILE"
grep -q "sockets" "$OVERRIDE_FILE"
grep -q "x11" "$OVERRIDE_FILE"
grep -q "devices" "$OVERRIDE_FILE"
grep -q "dri" "$OVERRIDE_FILE"
grep -q "shared" "$OVERRIDE_FILE"
grep -q "network" "$OVERRIDE_FILE"
grep -q "filesystems" "$OVERRIDE_FILE"
grep -q "home" "$OVERRIDE_FILE"
grep -q "\[Environment\]" "$OVERRIDE_FILE"
grep -q "FOO=bar" "$OVERRIDE_FILE"

echo "PASS: override-show"