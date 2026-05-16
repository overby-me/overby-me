# Test objdump -t on an archive — should print "In archive ARCH:" then per-member tables
mkdir -p "$TMPDIR/objs"
cp "$TESTOBJ" "$TMPDIR/objs/bintest.o"
ARCHIVE="$TMPDIR/objs/lib.a"
ar cr "$ARCHIVE" "$TMPDIR/objs/bintest.o"

(cd "$TMPDIR/objs" && "$REF" -t lib.a) > "$TMPDIR/expected" 2>&1 || true
(cd "$TMPDIR/objs" && "$RUST" -t lib.a) > "$TMPDIR/actual" 2>&1 || true
compare "objdump -t archive"
