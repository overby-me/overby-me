#!/bin/bash
set -euo pipefail

$FLATPAK build-init "$WORK/idemp" org.test.Idemp org.test.Sdk org.test.Platform
$FLATPAK build-finish "$WORK/idemp" --command app1 --share network
$FLATPAK build-finish "$WORK/idemp" --command app2 --socket x11

# command should be overwritten to app2
if ! grep -q "command=app2" "$WORK/idemp/metadata"; then
  echo "FAIL: command was not overwritten to app2"
  cat "$WORK/idemp/metadata"
  exit 1
fi

# network should still be there from first run
if ! grep -q "network" "$WORK/idemp/metadata"; then
  echo "FAIL: network share from first build-finish is missing"
  cat "$WORK/idemp/metadata"
  exit 1
fi

# x11 should be added
if ! grep -q "x11" "$WORK/idemp/metadata"; then
  echo "FAIL: x11 socket from second build-finish is missing"
  cat "$WORK/idemp/metadata"
  exit 1
fi

echo "PASS: build-finish-idempotent"