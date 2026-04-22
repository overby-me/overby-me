#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Create a file we can export.
mkdir -p "$WORK/docs"
echo "secret data" > "$WORK/docs/file1.txt"

out=$($FLATPAK document-export "$WORK/docs/file1.txt" 2>&1)
echo "export output: $out"
DOC_ID=$(echo "$out" | sed -n 's/^Exported as: //p' | head -1)
if [ -z "$DOC_ID" ]; then
  echo "FAIL: no doc id printed"
  exit 1
fi
ok "document exported with id: $DOC_ID"

# documents listing should now include this doc id.
list=$($FLATPAK documents 2>&1)
echo "list: $list"
if ! echo "$list" | grep -q "$DOC_ID"; then
  echo "FAIL: doc id missing from documents listing"
  echo "Got: $list"
  exit 1
fi
ok "doc id present in documents listing"

echo "PASS: vm-document-export"
