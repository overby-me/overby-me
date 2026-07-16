"$REF"  --color=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --color=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mon --color=foo (invalid color)"
