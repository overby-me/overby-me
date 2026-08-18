source ../helpers.nu

try { ^$env.REF foo o+e> ($env.TMPDIR | path join c.full) }
try { ^$env.RUST foo o+e> ($env.TMPDIR | path join r.full) }
# Filter ALSA-init noise C emits in the sandbox.
^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected)
^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual)
compare "spa-acp-tool foo (unknown command)"
