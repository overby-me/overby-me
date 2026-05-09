# Unknown option: pw-config prefixes the help with the standard
# `<argv0>: unrecognized option '<flag>'` message that getopt_long emits.
"$REF"  --bad-flag </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --bad-flag </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-config --bad-flag"
