#!/bin/bash
set -euo pipefail

# CVE-2021-43860: metadata containing NUL bytes must be rejected.
$FLATPAK build-init "$WORK/nul-app" org.test.NulMeta org.test.Sdk org.test.Platform stable
mkdir -p "$WORK/nul-app/files/bin"
echo '#!/bin/sh' > "$WORK/nul-app/files/bin/app"
chmod +x "$WORK/nul-app/files/bin/app"
$FLATPAK build-finish "$WORK/nul-app" --command app

# Inject NUL bytes
printf '[Application]\nname=org.test.NulMeta\nruntime=org.test.Platform/x86_64/stable\ncommand=app\n\0[Context]\nfilesystems=host;\n' > "$WORK/nul-app/metadata"

rc=0
output=$($FLATPAK --user install "$WORK/nul-app" 2>&1) || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: install should reject NUL-byte metadata"; exit 1; }

# Verify rejection mentions CVE or NUL
echo "$output" | grep -qiE "NUL|CVE-2021-43860|invalid" || {
  echo "FAIL: expected rejection message to mention NUL/CVE"
  echo "Got: $output"
  exit 1
}

echo "PASS: metadata-nul-byte-rejected"
