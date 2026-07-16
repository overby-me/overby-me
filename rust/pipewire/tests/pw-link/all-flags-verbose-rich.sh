"$REF"  -i -o -l -I -v </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -i -o -l -I -v </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-link -i -o -l -I -v (rich daemon, all flags + verbose)"
