#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that config --unset removes a key

# Set a value first
$FLATPAK --user config --set languages "en;de"
output=$($FLATPAK --user config --get languages 2>&1)
assert_streq "$output" "en;de"
ok "config --set languages works"

# Unset the value
$FLATPAK --user config --unset languages
ok "config --unset succeeded"

# Verify the key is gone (--get should fail)
rc=0
$FLATPAK --user config --get languages 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then
  ok "config --get fails after --unset"
else
  echo "FAIL: config --get should fail after --unset"
  exit 1
fi

# Unset of non-existent key should not fail
$FLATPAK --user config --unset nonexistent-key
ok "config --unset of non-existent key does not fail"

echo "PASS: vm-config-unset"