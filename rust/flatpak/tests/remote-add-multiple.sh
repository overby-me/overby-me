#!/bin/bash
set -euo pipefail

$FLATPAK --user remote-add remote1 https://example.com/repo1
$FLATPAK --user remote-add remote2 https://example.com/repo2
$FLATPAK --user remotes > "$WORK/remotes_out"
grep -q "remote1" "$WORK/remotes_out"
grep -q "remote2" "$WORK/remotes_out"

echo "PASS: remote-add-multiple"