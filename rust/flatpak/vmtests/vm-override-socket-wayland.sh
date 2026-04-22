#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
# Remove wayland socket first, then re-add via override
$FLATPAK --user override --nosocket=wayland org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "nosocket.*wayland|wayland"
ok "nosocket wayland override written"
# Now add it back
$FLATPAK --user override --socket=wayland org.test.Hello
assert_file_has_content "$OVERRIDE_FILE" "wayland"
ok "socket wayland override written"
echo "PASS: vm-override-socket-wayland"