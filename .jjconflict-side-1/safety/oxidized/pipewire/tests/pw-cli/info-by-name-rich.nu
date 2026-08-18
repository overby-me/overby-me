source ../helpers.nu

# Rich daemon: `info Node` should find the null-audio-sink Node by type
# substring (no other interfaces contain "Node" in this daemon; ClientNode
# isn't auto-created).
^$env.REF info Node o+e> ($env.TMPDIR | path join expected)
^$env.RUST info Node o+e> ($env.TMPDIR | path join actual)
compare "pw-cli info Node (rich daemon, by-name)"
