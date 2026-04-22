#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

mkdir -p "$WORK/docs"
echo "info-test" > "$WORK/docs/info.txt"
abs_path="$WORK/docs/info.txt"

DOC_ID=$($FLATPAK document-export "$abs_path" 2>&1 | sed -n 's/^Exported as: //p' | head -1)
ok "exported as $DOC_ID"

info=$($FLATPAK document-info "$DOC_ID" 2>&1)
echo "info: $info"
if ! echo "$info" | grep -q "ID: $DOC_ID"; then
  echo "FAIL: missing ID line"; exit 1
fi
if ! echo "$info" | grep -q "$abs_path"; then
  echo "FAIL: missing original path in info output"
  echo "Got: $info"
  exit 1
fi
ok "info reports original path"

echo "PASS: vm-document-info"
