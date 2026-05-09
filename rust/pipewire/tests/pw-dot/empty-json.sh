# pw-dot with an empty JSON file: emits the bare digraph header/footer
# and the standard "set output file -" prefix when -o is `-`.
echo '[]' > "$TMPDIR/test.json"
"$REF"  -j "$TMPDIR/test.json" -o - </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -j "$TMPDIR/test.json" -o - </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-dot/empty-json"
