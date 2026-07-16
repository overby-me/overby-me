# getopt: required-arg short flag accepts attached value `-u<val>`.
"$REF"  -u0.75 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -u0.75 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -u0.75 (attached short value → '0.750000')"
