#!/bin/bash
set -euo pipefail

cat > "$WORK/test2.flatpakrepo" << 'EOF'
[Flatpak Repo]
Title=Alt Remote
Url=https://alt.example.com/repo
EOF

$FLATPAK --user remote-add --from "$WORK/test2.flatpakrepo"

output=$($FLATPAK --user remotes 2>&1)

if ! echo "$output" | grep -q "test2"; then
  echo "FAIL: remote 'test2' not found in remotes output"
  echo "$output"
  exit 1
fi

echo "PASS: remote-add-flatpakrepo-url"