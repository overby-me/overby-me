"$REF"  -rfoo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -rfoo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-metadata -rfoo (bad remote)"
