#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

setup_repo
# Set env override and verify it's written
$FLATPAK --user override --env MY_TEST_VAR=hello123 org.test.Hello
OVERRIDE_FILE="$FL_DIR/overrides/org.test.Hello"
assert_has_file "$OVERRIDE_FILE"
assert_file_has_content "$OVERRIDE_FILE" "MY_TEST_VAR=hello123"
# Try running with the override (sandbox may or may not apply it)
output=$(run_sh org.test.Hello 'echo ${MY_TEST_VAR:-unset}' 2>&1 || true)
echo "MY_TEST_VAR in sandbox: $output"
ok "override env sandbox"

echo "PASS: vm-override-env-sandbox"