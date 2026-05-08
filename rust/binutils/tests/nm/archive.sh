# Test nm on an archive — should print "<member.o>:" header per member
mkdir -p "$TMPDIR/objs"
cp "$TESTOBJ" "$TMPDIR/objs/bintest.o"
ARCHIVE="$TMPDIR/objs/lib.a"
ar cr "$ARCHIVE" "$TMPDIR/objs/bintest.o"

(cd "$TMPDIR/objs" && "$REF" lib.a) > "$TMPDIR/expected" 2>&1 || true
(cd "$TMPDIR/objs" && "$RUST" lib.a) > "$TMPDIR/actual" 2>&1 || true
compare "nm archive"
