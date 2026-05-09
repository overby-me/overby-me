"$REF"  load-module foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" load-module foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli load-module foo (could not load)"
