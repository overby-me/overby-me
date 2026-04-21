#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
$FLATPAK --user override --nofilesystem=host org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "host"
ok "nofilesystem host override written"
echo "PASS: vm-override-nofilesystem-host"