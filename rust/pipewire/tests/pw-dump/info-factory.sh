# Factory info block: name, type, version (uint32 not string), change-mask, props.
# Factory id 7 is libpipewire-module-metadata factory (always present).
"$REF"  7 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" 7 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dump 7 (Factory info block: name/type/version/props)"
