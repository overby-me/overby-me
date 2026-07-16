"$REF"  --color </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --color </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link --color (rejected — not a valid option)"
