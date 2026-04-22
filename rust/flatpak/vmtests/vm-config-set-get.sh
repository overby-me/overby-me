#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test config --set / --get / --unset

# Set a config value
$FLATPAK --user config --set languages "en;de"
ok "config --set succeeded"

# Get the config value back
output=$($FLATPAK --user config --get languages 2>&1)
assert_streq "$output" "en;de"
ok "config --get returns correct value"

# Overwrite the value
$FLATPAK --user config --set languages "en;fr"
output=$($FLATPAK --user config --get languages 2>&1)
assert_streq "$output" "en;fr"
ok "config --set overwrites existing value"

# Set a second key
$FLATPAK --user config --set extra-languages "de"
output=$($FLATPAK --user config --get extra-languages 2>&1)
assert_streq "$output" "de"
ok "config --set second key works"

# First key still intact
output=$($FLATPAK --user config --get languages 2>&1)
assert_streq "$output" "en;fr"
ok "first key still readable after setting second"

# Unset the first key
$FLATPAK --user config --unset languages
rc=0
$FLATPAK --user config --get languages 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then
  ok "config --unset removes key (--get fails)"
else
  echo "FAIL: config --get should fail after --unset"
  exit 1
fi

# Second key still intact after unset of first
output=$($FLATPAK --user config --get extra-languages 2>&1)
assert_streq "$output" "de"
ok "other keys survive --unset"

echo "PASS: vm-config-set-get"