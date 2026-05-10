"$REF"  /proc/cpuinfo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" /proc/cpuinfo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump /proc/cpuinfo (stat size=0 → mmap EINVAL)"
