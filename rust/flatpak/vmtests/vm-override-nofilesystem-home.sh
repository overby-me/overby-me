#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
$FLATPAK --user override --nofilesystem=home org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "home"
ok "nofilesystem home override written"
echo "PASS: vm-override-nofilesystem-home"