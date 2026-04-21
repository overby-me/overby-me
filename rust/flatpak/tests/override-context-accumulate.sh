#!/bin/bash
set -euo pipefail

$FLATPAK --user override --socket x11 org.test.Accum
$FLATPAK --user override --socket wayland org.test.Accum
$FLATPAK --user override --socket pulseaudio org.test.Accum

OVERRIDE_FILE="$HOME/.local/share/flatpak/overrides/org.test.Accum"

# The sockets field should contain all three, semicolon separated
content=$(grep "^sockets=" "$OVERRIDE_FILE" || cat "$OVERRIDE_FILE")
echo "$content" | grep -q "x11"
echo "$content" | grep -q "wayland"
echo "$content" | grep -q "pulseaudio"

echo "PASS: override-context-accumulate"