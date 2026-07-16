# Rich daemon: `info Node` should find the null-audio-sink Node by type
# substring (no other interfaces contain "Node" in this daemon — ClientNode
# isn't auto-created).
"$REF"  info Node </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" info Node </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli info Node (rich daemon, by-name)"
