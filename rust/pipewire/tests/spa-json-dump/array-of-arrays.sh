cat > "$TMPDIR/in.json" <<'INNER'
matrix = [ [ 1 2 3 ] [ 4 5 6 ] [ 7 8 9 ] ]
INNER
"$REF"  "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/array-of-arrays"
