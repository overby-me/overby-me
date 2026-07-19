source ../helpers.nu

try { ^$env.REF card o+e> ($env.TMPDIR | path join c.full) }
try { ^$env.RUST card o+e> ($env.TMPDIR | path join r.full) }
# Filter ALSA-init noise C emits in the sandbox where no real cards
# are present (do_probe runs before the command and logs warnings).
try { ^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected) }
try { ^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual) }
compare "spa-acp-tool/card-no-args"
