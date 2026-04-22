#!/bin/bash
set -euo pipefail

export DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent

mkdir -p "$WORK/docs"
echo data > "$WORK/docs/a.txt"
abs="$WORK/docs/a.txt"

ID=$($FLATPAK document-export "$abs" 2>&1 | sed -n 's/^Exported as: //p' | head -1)
[ -n "$ID" ] || { echo "FAIL: empty doc id"; exit 1; }

info=$($FLATPAK document-info "$ID" 2>&1)
echo "$info" | grep -q "$abs" || { echo "FAIL: info missing path"; echo "$info"; exit 1; }

$FLATPAK document-unexport "$ID"
out=$($FLATPAK document-info "$ID" 2>&1) && { echo "FAIL: info should fail"; exit 1; } || true

echo "PASS: document-local-store"
