#!/bin/bash
set -euo pipefail

$FLATPAK --help > /dev/null 2>&1

echo "PASS: help-exit-zero"