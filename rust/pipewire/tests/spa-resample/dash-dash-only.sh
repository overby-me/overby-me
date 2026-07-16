"$REF"  -- > "$TMPDIR/expected" 2>&1 || true
"$RUST" -- > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -- (-- terminator, no positionals)"
