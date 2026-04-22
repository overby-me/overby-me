#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

mkdir -p "$WORK/docs"
echo "u" > "$WORK/docs/u.txt"
DOC_ID=$($FLATPAK document-export "$WORK/docs/u.txt" 2>&1 | sed -n 's/^Exported as: //p' | head -1)
ok "exported as $DOC_ID"

$FLATPAK document-unexport "$DOC_ID" 2>&1
ok "unexport succeeded"

# Subsequent info should fail.
set +e
out=$($FLATPAK document-info "$DOC_ID" 2>&1)
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
  echo "FAIL: document-info should fail after unexport"
  echo "Got: $out"
  exit 1
fi
ok "document-info errors after unexport"

# Listing should not include this doc id.
list=$($FLATPAK documents 2>&1)
if echo "$list" | grep -q "$DOC_ID"; then
  echo "FAIL: $DOC_ID still listed after unexport"
  exit 1
fi
ok "doc id removed from listing"

echo "PASS: vm-document-unexport"
