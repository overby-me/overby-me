# C: case 'u' → strtod(optarg, NULL) then fprintf(stderr, "%f\n", val).
"$REF"  --cutoff=0.5 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=0.5 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=0.5 (strtod → fprintf %f = '0.500000')"
