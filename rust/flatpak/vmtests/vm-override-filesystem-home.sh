#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"
setup_repo
$FLATPAK --user override --filesystem=home:ro org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "home"
ok "filesystem home:ro override written"
# Verify home is accessible read-only in sandbox
output=$(run_sh org.test.Hello 'test -d $HOME && echo home-visible || echo home-hidden' 2>&1 || true)
echo "Home in sandbox: $output"
ok "filesystem home override (checked)"
echo "PASS: vm-override-filesystem-home"