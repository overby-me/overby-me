#!/bin/bash
set -euo pipefail

. "${BASH_SOURCE%/*}/libtest-nix.sh"

setup_repo

# Setting --persist with path traversal must be rejected (CVE-2024-42472).
# The override is accepted (stored in overrides file), but at sandbox setup
# time the dangerous path is rejected with a warning and not bind-mounted.
$FLATPAK --user override --persist=../../../etc org.test.Hello

output=$(run_sh org.test.Hello "true" 2>&1)

assert_match "$output" "ignoring dangerous --persist path|persist.*reject" "path traversal via --persist should be blocked"

ok "persist path traversal blocked"
echo "PASS: vm-persist-path-traversal"
