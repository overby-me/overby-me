# Real upstream config: src/daemon/pipewire-aes67.conf.in. Exercises the
# dotted-quad-as-string requoting (e.g. 239.255.255.255 → "239.255.255.255").
"$REF" "$SRC/src/daemon/pipewire-aes67.conf.in" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/pipewire-aes67.conf.in" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-aes67"
