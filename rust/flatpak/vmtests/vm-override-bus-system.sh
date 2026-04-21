#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
$FLATPAK --user override --system-own-name=org.test.SysOwned --system-talk-name=org.test.SysTalked org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "org.test.SysOwned"
assert_file_has_content "$OVERRIDE_FILE" "org.test.SysTalked"
ok "system bus name overrides written"
echo "PASS: vm-override-bus-system"