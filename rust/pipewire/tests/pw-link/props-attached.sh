"$REF"  -pfoo bar baz </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -pfoo bar baz </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link -pfoo bar baz (attached value)"
