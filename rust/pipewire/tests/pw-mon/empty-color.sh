# pw-mon --color= (with `=` and empty value). C's getopt sets optarg
# to the empty string in this case; the value doesn't match any of
# 'auto'/'never'/'always' so we hit the "Invalid color: " error.
# Bare `--color` (no `=`) is different — optarg is NULL there and the
# C code's `optarg == NULL || strcmp(optarg, "auto")==0` short-circuits.
"$REF"  --color= </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --color= </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mon --color= (empty value rejected like C)"
