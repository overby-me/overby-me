SC_ID=$("$REF" ls SecurityContext </dev/null 2>/dev/null | awk '/^\tid /{print $2; exit}' | tr -d ',')
if [ -z "$SC_ID" ]; then
  echo "FAIL: no SecurityContext global"
  exit 1
fi
"$REF"  info "$SC_ID" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info "$SC_ID" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info <SecurityContext> (rich daemon)"
