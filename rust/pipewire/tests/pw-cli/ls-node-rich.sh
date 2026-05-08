# Rich daemon: pre-loaded with a null-audio-sink Node. ls Node should find
# at least one entry; both binaries should produce identical output.
"$REF"  ls Node </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" ls Node </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli ls Node (rich daemon)"
