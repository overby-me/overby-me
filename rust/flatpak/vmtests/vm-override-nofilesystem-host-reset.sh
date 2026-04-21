#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
# First add a filesystem override, then reset it
$FLATPAK --user override --filesystem=home org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
$FLATPAK --user override --nofilesystem=host:reset org.test.Hello
assert_file_has_content "$OVERRIDE_FILE" "host:reset\|reset"
ok "nofilesystem host:reset override written"
echo "PASS: vm-override-nofilesystem-host-reset"