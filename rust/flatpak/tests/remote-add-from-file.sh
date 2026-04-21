#!/bin/bash
set -euo pipefail

mkdir -p "$HOME/.local/share/flatpak"

cat > "$WORK/test.flatpakrepo" << 'EOF'
[Flatpak Repo]
Title=Test Remote
Url=https://example.com/test-repo
EOF

$FLATPAK --user remote-add --from "$WORK/test.flatpakrepo"

REMOTES_DIR="$HOME/.local/share/flatpak"

if ! find "$REMOTES_DIR" -type f -exec grep -l "https://example.com/test-repo" {} + >/dev/null 2>&1; then
  echo "FAIL: remote URL 'https://example.com/test-repo' not found in flatpak config under $REMOTES_DIR"
  exit 1
fi

echo "PASS: remote-add-from-file"