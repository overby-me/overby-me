"$REF"  help -rfoo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" help -rfoo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli help -rfoo (startup connect with bad remote)"
