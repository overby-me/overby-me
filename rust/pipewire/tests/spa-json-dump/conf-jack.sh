# Real upstream config: src/daemon/jack.conf.in.
"$REF" "$SRC/src/daemon/jack.conf.in" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/jack.conf.in" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-jack"
