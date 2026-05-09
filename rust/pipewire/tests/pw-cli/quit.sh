# `pw-cli quit` / `q` produces no stdout, no stderr, exits 0.
"$REF"  quit </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" quit </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli quit (silent exit)"
