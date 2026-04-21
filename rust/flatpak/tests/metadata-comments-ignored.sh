#!/bin/bash
set -euo pipefail

cat > "$WORK/test.meta" << 'EOF'
# This is a comment
; Another comment
[Application]
name=org.test.Comments
runtime=org.test.Platform/x86_64/master

[Context]
# permission comment
shared=network
EOF
$FLATPAK build-init "$WORK/commentapp" org.test.Comments org.test.Sdk org.test.Platform
# Overwrite metadata with our version
cp "$WORK/test.meta" "$WORK/commentapp/metadata"
$FLATPAK build-finish "$WORK/commentapp" --command test --socket x11
# Check sockets were added (metadata was parsed correctly despite comments)
grep -q "sockets" "$WORK/commentapp/metadata"
grep -q "x11" "$WORK/commentapp/metadata"

echo "PASS: metadata-comments-ignored"