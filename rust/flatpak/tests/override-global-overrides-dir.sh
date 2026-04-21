#!/bin/bash
set -euo pipefail

# Start with no overrides dir
rm -rf "$HOME/.local/share/flatpak/overrides"
$FLATPAK --user override --socket x11 org.test.GlobalDir
[ -d "$HOME/.local/share/flatpak/overrides" ]
[ -f "$HOME/.local/share/flatpak/overrides/org.test.GlobalDir" ]

echo "PASS: override-global-overrides-dir"