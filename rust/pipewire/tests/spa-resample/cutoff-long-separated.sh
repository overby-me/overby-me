# getopt_long: long flag with separated value `--cutoff <val>`.
"$REF"  --cutoff 0.99 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff 0.99 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff 0.99 (separated long value → '0.990000')"
