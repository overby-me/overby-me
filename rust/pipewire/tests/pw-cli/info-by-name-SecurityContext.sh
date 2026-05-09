# Look up first global of type `SecurityContext` by substring match.
"$REF"  info SecurityContext </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info SecurityContext </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info SecurityContext (by-name)"
