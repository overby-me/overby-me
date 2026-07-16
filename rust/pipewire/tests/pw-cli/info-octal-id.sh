# strtoul base 0 with leading `0` means octal. `info 010` = id 8.
"$REF"  "info 010" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" "info 010" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info 010 (octal id parsing)"
