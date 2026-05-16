# Core info block: cookie, user-name, host-name, version, name, change-mask, props.
# Cookie and hostname vary per run/host so we normalize them.
"$REF"  0 </dev/null > "$TMPDIR/c.full" 2>&1
"$RUST" 0 </dev/null > "$TMPDIR/r.full" 2>&1
# Normalize cookie (uint32) and host-name/user-name (vary by host).
normalize() {
  sed -E '
    s|"cookie": [0-9]+|"cookie": COOKIE|;
    s|"user-name": "[^"]*"|"user-name": "USER"|;
    s|"host-name": "[^"]*"|"host-name": "HOST"|;
  ' "$1" > "$2"
}
normalize "$TMPDIR/c.full" "$TMPDIR/expected"
normalize "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-dump 0 (Core info block: cookie/user-name/host-name/version/name/props)"
