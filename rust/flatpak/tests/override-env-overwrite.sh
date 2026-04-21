#!/bin/bash
set -euo pipefail

"$FLATPAK" --user override --env FOO=first org.test.EnvOW
"$FLATPAK" --user override --env FOO=second org.test.EnvOW

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.EnvOW"

if [ ! -f "$OVERRIDE_FILE" ]; then
  echo "FAIL: override file not created at $OVERRIDE_FILE"
  exit 1
fi

if grep -q "FOO=first" "$OVERRIDE_FILE"; then
  echo "FAIL: override file still contains FOO=first (should have been overwritten)"
  cat "$OVERRIDE_FILE"
  exit 1
fi

if ! grep -q "FOO=second" "$OVERRIDE_FILE"; then
  echo "FAIL: override file does not contain FOO=second"
  cat "$OVERRIDE_FILE"
  exit 1
fi

echo "PASS: override-env-overwrite"