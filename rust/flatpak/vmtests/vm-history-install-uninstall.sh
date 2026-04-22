#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

setup_repo

# Verify history shows the install event
output=$($FLATPAK --user history 2>&1)
echo "history after install: $output"

if echo "$output" | grep -q "install"; then
  ok "history records install event"
else
  echo "FAIL: history does not show install event"
  echo "Got: $output"
  exit 1
fi

if echo "$output" | grep -q "org.test.Hello"; then
  ok "history contains app ref"
else
  echo "FAIL: history does not contain org.test.Hello"
  echo "Got: $output"
  exit 1
fi

# Uninstall the app
$FLATPAK --user uninstall org.test.Hello 2>&1

# Verify history shows uninstall event
output=$($FLATPAK --user history 2>&1)
echo "history after uninstall: $output"

if echo "$output" | grep -q "uninstall"; then
  ok "history records uninstall event"
else
  echo "FAIL: history does not show uninstall event"
  echo "Got: $output"
  exit 1
fi

echo "PASS: vm-history-install-uninstall"