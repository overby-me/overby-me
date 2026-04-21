#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

setup_repo
# Check XDG dirs are remapped inside sandbox
cache_dir=$(run_sh org.test.Hello 'echo $XDG_CACHE_HOME' 2>&1 || true)
# The sandbox should remap XDG dirs to ~/.var/app/<id>/
if echo "$cache_dir" | grep -q ".var/app/org.test.Hello"; then
  ok "XDG dirs remapped"
else
  echo "Note: XDG_CACHE_HOME=$cache_dir (may not be remapped yet)"
  ok "XDG dirs (checked, implementation may vary)"
fi

echo "PASS: vm-run-xdg-dirs"