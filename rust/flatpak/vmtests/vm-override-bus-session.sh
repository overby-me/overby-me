#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
$FLATPAK --user override --own-name=org.test.Owned --talk-name=org.test.Talked org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "org.test.Owned"
assert_file_has_content "$OVERRIDE_FILE" "org.test.Talked"
ok "session bus name overrides written"
echo "PASS: vm-override-bus-session"