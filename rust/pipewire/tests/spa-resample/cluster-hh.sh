"$REF"  -hh </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -hh </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample/cluster-hh"
