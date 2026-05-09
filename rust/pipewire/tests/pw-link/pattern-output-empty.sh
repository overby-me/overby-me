# pw-link -o with a pattern that doesn't match any output port produces
# no output (rich daemon has no output ports anyway).
"$REF"  -o nonexistent </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -o nonexistent </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-link -o nonexistent (rich daemon, empty)"
