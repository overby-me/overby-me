"$REF"  foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link foo (connect mode, missing input)"
