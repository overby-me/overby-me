"$REF"  -- </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -- </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump -- (lone -- terminator → live mode connect-fail)"
