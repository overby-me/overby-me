# Basic daemon: pw-metadata --list against a daemon that loads
# libpipewire-module-metadata still has the daemon-side metadata
# instances; both binaries should produce the same list.
"$REF"  --list </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" --list </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-metadata --list (basic daemon)"
