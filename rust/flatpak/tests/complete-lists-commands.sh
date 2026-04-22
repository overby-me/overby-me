#!/bin/bash
set -euo pipefail

out=$("$FLATPAK" complete 2>&1)
for cmd in install uninstall list run; do
  if ! echo "$out" | grep -qx "$cmd"; then
    echo "FAIL: 'flatpak complete' missing '$cmd'"
    echo "$out"
    exit 1
  fi
done

out=$("$FLATPAK" complete inst 2>&1)
if ! echo "$out" | grep -qx "install"; then
  echo "FAIL: 'flatpak complete inst' missing 'install'"
  echo "$out"
  exit 1
fi
if echo "$out" | grep -qx "run"; then
  echo "FAIL: 'flatpak complete inst' should not list 'run'"
  echo "$out"
  exit 1
fi

echo "PASS: complete-lists-commands"
