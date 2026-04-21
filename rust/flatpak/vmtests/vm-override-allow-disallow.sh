#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
$FLATPAK --user override --allow=multiarch org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "multiarch"
ok "allow multiarch override written"
$FLATPAK --user override --disallow=multiarch org.test.Hello
assert_file_has_content "$OVERRIDE_FILE" "multiarch"
ok "disallow multiarch override written"
echo "PASS: vm-override-allow-disallow"