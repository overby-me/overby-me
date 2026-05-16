# Test nm --reverse-sort --numeric-sort — combined sort flags
$REF --reverse-sort --numeric-sort "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --reverse-sort --numeric-sort "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --reverse-sort --numeric-sort"
