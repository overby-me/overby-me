# C silently consumes only first 2 positional; extras are ignored.
"$REF"  a b c </dev/null > "$TMPDIR/c.full" 2>&1 || true
"$RUST" a b c </dev/null > "$TMPDIR/r.full" 2>&1 || true
# Strip the connect-attempt error (which depends on having a daemon).
grep -v "failed to link ports" "$TMPDIR/c.full" > "$TMPDIR/expected"
grep -v "failed to link ports" "$TMPDIR/r.full" > "$TMPDIR/actual"
compare "pw-link a b c (extra positional ignored)"
