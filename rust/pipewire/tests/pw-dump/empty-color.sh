# pw-dump --color= (empty value via `=`). C's getopt sets optarg to ""
# for `--color=`; the empty value doesn't match auto/never/always so we
# hit the "Unknown color: " error path. Distinct from bare `--color`
# which uses the auto default.
"$REF"  --color= </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --color= </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dump --color= (empty value → 'Unknown color: ')"
