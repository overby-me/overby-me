# pw-cli set-param with no args prints `Error: "set-param <usage>"` to stderr.
"$REF"  set-param </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" set-param </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli set-param (usage error)"
