# pw-link -l -I -v on a daemon with no links: produces no output
# (basic daemon).
"$REF"  -l -I -v </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -l -I -v </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -l -I -v (basic daemon, empty)"
