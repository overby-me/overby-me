# pw-link -t alone (without -i/-o/-l): C does nothing — no per-port
# iteration since neither LIST_PORTS nor LIST_LINKS is set.
"$REF"  -t </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -t </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -t alone (no output)"
