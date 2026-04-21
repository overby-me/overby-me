#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/dirapp" org.test.Dirs org.test.Sdk org.test.Platform

for d in files files/bin files/lib files/share var var/tmp var/lib var/run; do
  if [ ! -d "$WORK/dirapp/$d" ]; then
    echo "FAIL: expected directory $d not created"
    exit 1
  fi
done

echo "PASS: build-init-dirs"