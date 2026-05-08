# Rich daemon: a null-audio-sink Node lives at id 8 (after Core/0,
# Module/1, SecurityContext/2, Module/3, Factory/4, Module/5, Metadata/6,
# Module/7-spa-node-factory, Node/8).
#
# Both binaries should bind the node and print the same info, params, props.
"$REF"  info 8 </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" info 8 </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli info 8 (Node, rich daemon)"
