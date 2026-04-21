#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
$FLATPAK --user override --nodevice=dri org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "dri"
ok "nodevice dri override written"
$FLATPAK --user override --device=dri org.test.Hello
assert_file_has_content "$OVERRIDE_FILE" "dri"
ok "device dri override written"
echo "PASS: vm-override-device-dri"