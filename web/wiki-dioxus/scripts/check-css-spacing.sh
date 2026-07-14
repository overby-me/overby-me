#!/usr/bin/env bash
# Spacing/type-size lint gate for assets/style.css (design system #1).
#
# Spacing and font-size must flow through the design tokens
# (--md-sys-spacing-* and the typescale size tokens) so a global density or
# type-scale change is one edit, not a find-and-replace across hundreds of
# literals. There is a large existing backlog of raw-px literals, so this is a
# RATCHET, not a hard ban: the count may only ever go DOWN. When you migrate a
# literal to a token, lower BASELINE to match; never raise it.
#
# Wire into CI / pre-commit (fails the build if the count regresses):
#   web/wiki-dioxus/scripts/check-css-spacing.sh
set -euo pipefail

css="$(cd "$(dirname "$0")/.." && pwd)/assets/style.css"

# The current sanctioned ceiling. Lower this as literals are migrated to tokens.
BASELINE=339

count=$(grep -oE \
  '(padding|margin|margin-top|margin-bottom|margin-left|margin-right|gap|font-size)[^;:]*:[^;]*[0-9]+px' \
  "$css" | wc -l | tr -d ' ')

echo "raw-px spacing/font-size declarations in style.css: $count (ceiling $BASELINE)"

if [ "$count" -gt "$BASELINE" ]; then
  echo
  echo "ERROR: raw-px spacing/font-size rose above the ceiling of $BASELINE."
  echo "Use the tokens instead of literal px:"
  echo "  padding/margin/gap -> var(--md-sys-spacing-N)   (N: 1=4px 2=8px 3=12px 4=16px 5=20px 6=24px)"
  echo "  font-size          -> the typescale size tokens in m3-tokens.css"
  exit 1
fi

if [ "$count" -lt "$BASELINE" ]; then
  echo "Nice — literals dropped below the ceiling. Lower BASELINE to $count in this script."
fi
