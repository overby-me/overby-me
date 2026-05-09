# Unrecognized option: getopt_long emits standard `<argv0>: unrecognized
# option '<flag>'` to stderr, then the help block to stdout.
"$REF"  --bad-flag </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --bad-flag </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-metadata --bad-flag"
