# Real upstream config: src/daemon/minimal.conf.in.
"$REF" "$SRC/src/daemon/minimal.conf.in" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/minimal.conf.in" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-minimal"
