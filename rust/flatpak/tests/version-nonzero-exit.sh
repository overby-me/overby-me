#!/bin/bash
set -euo pipefail

$FLATPAK --version > /dev/null 2>&1
# implicit: if it exits non-zero, set -e will catch it

echo "PASS: version-nonzero-exit"