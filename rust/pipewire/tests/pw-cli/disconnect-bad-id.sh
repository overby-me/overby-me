# C's do_disconnect: when given a remote id not in the table,
# find_var(idx, TYPE_REMOTE) returns NULL → error
# `Remote <idx> does not exist`. Our non-REPL mode only has id 0, so
# any non-zero id should hit this error path. Daemon connection still
# happens first (every pw-cli command connects before parsing).
"$REF"  "disconnect 99" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" "disconnect 99" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli disconnect 99 (Error: 'Remote 99 does not exist')"
