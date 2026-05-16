# `pw-cat --dsd -p` switches to TYPE_DSD; C uses dsdfile loader.
"$REF"  --dsd -p /tmp/nonexistent-dsd-file > "$TMPDIR/expected" 2>&1 || true
"$RUST" --dsd -p /tmp/nonexistent-dsd-file > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --dsd -p (uses dsdfile error format)"
