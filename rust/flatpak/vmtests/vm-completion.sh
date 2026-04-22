#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Test that the `complete` subcommand prints expected candidates.

out=$($FLATPAK complete 2>&1)
echo "complete (no args) output:"
echo "$out"
for cmd in install run list; do
  if ! echo "$out" | grep -qx "$cmd"; then
    echo "FAIL: 'flatpak complete' missing '$cmd'"
    exit 1
  fi
done
ok "flatpak complete includes install, run, list"

out=$($FLATPAK complete remote- 2>&1)
echo "complete remote- output:"
echo "$out"
for cmd in remote-add remote-delete remote-ls remote-info; do
  if ! echo "$out" | grep -qx "$cmd"; then
    echo "FAIL: 'flatpak complete remote-' missing '$cmd'"
    exit 1
  fi
done
if echo "$out" | grep -qx "install"; then
  echo "FAIL: 'flatpak complete remote-' should not list 'install'"
  exit 1
fi
ok "flatpak complete remote- filters to remote-* only"

echo "PASS: vm-completion"
