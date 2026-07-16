# getopt: short flag with separated value `-u <val>`.
"$REF"  -u 0.42 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -u 0.42 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -u 0.42 (separated short value → '0.420000')"
