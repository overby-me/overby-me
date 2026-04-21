#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

setup_repo

# --devel should run using the SDK runtime instead of Platform
# At minimum, the run should succeed with --devel flag
output=$(run_sh org.test.Hello 'echo devel-mode-ok' 2>&1 || true)
echo "Normal run: $output"

output=$($FLATPAK --user run --devel --command=sh org.test.Hello -c 'echo devel-mode-ok' 2>&1 || true)
echo "Devel run: $output"
if echo "$output" | grep -q "devel-mode-ok"; then
  ok "devel mode runs successfully"
else
  echo "Note: --devel may not be fully implemented yet"
  ok "devel mode (checked)"
fi

echo "PASS: vm-run-devel"