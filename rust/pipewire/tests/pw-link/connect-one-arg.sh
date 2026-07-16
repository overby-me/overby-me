"$REF"  a </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" a </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link a (1 positional → error)"
