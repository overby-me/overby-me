#!/bin/bash
set -euo pipefail

# Commands that accept --help or at least don't crash when called with it.
# We verify the binary doesn't segfault (exits with some code) for each command.
# Some commands (like ps, kill, history) don't support --help and may produce
# no output — that's fine as long as they don't crash.

commands=(
  run list info install uninstall update
  override remotes remote-add remote-delete remote-ls
  ps kill enter search history config repair
  build-init build build-finish build-export build-bundle
  build-import-bundle build-sign build-update-repo build-commit-from
  repo create-usb mask pin
)

for cmd in "${commands[@]}"; do
  # Run the command with --help, allow non-zero exit
  rc=0
  output=$("$FLATPAK" "$cmd" --help 2>&1) || rc=$?

  # A segfault gives rc=139. Anything else is acceptable.
  if [ "$rc" -eq 139 ] || [ "$rc" -eq 134 ] || [ "$rc" -eq 136 ]; then
    echo "FAIL: '$cmd --help' crashed with signal (rc=$rc)"
    exit 1
  fi

  echo "OK: $cmd --help exited with rc=$rc"
done

echo "PASS: all subcommand --help calls completed without crashing"