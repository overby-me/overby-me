"$REF"  -m </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -m </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat -m alone (sub-mode requires -p or -r)"
