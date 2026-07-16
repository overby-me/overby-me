# C silently consumes only first 2 positional; extras are ignored. In
# the no-daemon sandbox both binaries emit "can't connect: Host is down".
"$REF"  a b c </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" a b c </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link a b c (extra positional ignored)"
