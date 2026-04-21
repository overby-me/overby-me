#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user remote-add dup-remote https://example.com/repo

rc=0
output=$($FLATPAK --user remote-add dup-remote https://example.com/other 2>&1) || rc=$?

if [ "$rc" -eq 0 ]; then
  echo "FAIL: expected non-zero exit when adding duplicate remote"
  exit 1
fi

if ! echo "$output" | grep -qi "already exists"; then
  echo "FAIL: output does not mention 'already exists': $output"
  exit 1
fi

echo "PASS: remote-add-duplicate"