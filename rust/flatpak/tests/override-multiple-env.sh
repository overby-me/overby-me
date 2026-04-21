#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

$FLATPAK --user override --env FOO=bar org.test.Multi
$FLATPAK --user override --env BAZ=qux org.test.Multi
$FLATPAK --user override --env EMPTY= org.test.Multi

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Multi"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "FOO=bar" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'FOO=bar'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "BAZ=qux" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'BAZ=qux'"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "EMPTY=" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain 'EMPTY='"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-multiple-env"