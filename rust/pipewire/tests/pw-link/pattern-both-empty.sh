# pw-link with -o pattern -i pattern: with non-matching patterns, both
# directions are filtered to empty.
"$REF"  -o nonexistent -i nonexistent </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -o nonexistent -i nonexistent </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-link -o nonexistent -i nonexistent"
