# Test ar tO — list members with their archive file offsets
mkdir -p "$TMPDIR/objs"
gcc -c -x c - <<<'int foo(int x){return x+1;}' -o "$TMPDIR/objs/foo.o"
gcc -c -x c - <<<'int bar(int x){return x*2;}' -o "$TMPDIR/objs/bar.o"
ARCHIVE="$TMPDIR/objs/libtest.a"
"$REF" cr "$ARCHIVE" "$TMPDIR/objs/foo.o" "$TMPDIR/objs/bar.o"

(cd "$TMPDIR/objs" && "$REF" tO libtest.a) > "$TMPDIR/expected" 2>&1 || true
(cd "$TMPDIR/objs" && "$RUST" tO libtest.a) > "$TMPDIR/actual" 2>&1 || true
compare "ar tO"
