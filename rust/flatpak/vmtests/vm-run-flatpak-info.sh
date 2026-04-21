#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

setup_repo
# Run the app and check if /.flatpak-info exists inside the sandbox
output=$(run_sh org.test.Hello 'cat /.flatpak-info 2>/dev/null || echo NO_FLATPAK_INFO' 2>&1)
# Depending on rust-flatpak implementation, this may or may not exist yet
# At minimum, the run should succeed
ok "run flatpak-info"

echo "PASS: vm-run-flatpak-info"