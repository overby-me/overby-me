# pw-config --color= (empty value via `=`). Same C behavior as pw-dump:
# empty optarg rejected because it isn't auto/never/always.
"$REF"  --color= </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --color= </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-config --color= (empty value → 'Unknown color: ')"
