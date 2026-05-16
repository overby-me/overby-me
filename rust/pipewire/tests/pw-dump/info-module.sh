# pw-dump <module-id> should emit the per-class "info" block:
# Module's info has name, filename, args, change-mask flags, and a
# sorted props dict.
"$REF"  1 </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/c.err"
"$RUST" 1 </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/r.err"
# Normalize the absolute module path under /nix/store (varies per
# pipewire derivation revision).
sed -i -E 's|/nix/store/[a-z0-9]{32}-[^"]*pipewire-[^"]+/lib/pipewire-[^/]+/|NIXLIBPATH/|g' \
  "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-dump 1 (Module info block: name, filename, args, change-mask, sorted props)"
