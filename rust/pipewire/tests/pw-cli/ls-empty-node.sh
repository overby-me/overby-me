# In a stripped-down daemon (no audio backends, no monitors), there should
# be no Node globals — both binaries should print exactly nothing.
"$REF"  ls Node </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" ls Node </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli ls Node (none)"
