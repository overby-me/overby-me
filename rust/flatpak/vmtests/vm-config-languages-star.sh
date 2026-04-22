#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that setting languages to "*" works correctly

$FLATPAK --user config --set languages "*"
ok "config --set languages '*' succeeded"

output=$($FLATPAK --user config --get languages 2>&1)
assert_streq "$output" "*"
ok "config --get languages returns '*'"

echo "PASS: vm-config-languages-star"