"$REF"  -Cnever </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -Cnever </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mon -Cnever (-C is no-arg, -n unknown)"
