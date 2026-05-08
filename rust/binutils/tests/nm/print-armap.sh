# Test nm -s on an archive — print archive index then member symbols
mkdir -p "$TMPDIR/objs"
cp "$TESTOBJ" "$TMPDIR/objs/bintest.o"
ARCHIVE="$TMPDIR/objs/lib.a"
ar cr "$ARCHIVE" "$TMPDIR/objs/bintest.o"

(cd "$TMPDIR/objs" && "$REF" -s lib.a) > "$TMPDIR/expected" 2>&1 || true
(cd "$TMPDIR/objs" && "$RUST" -s lib.a) > "$TMPDIR/actual" 2>&1 || true
compare "nm -s archive"
