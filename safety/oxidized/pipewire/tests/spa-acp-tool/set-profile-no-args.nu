source ../helpers.nu

try { ^$env.REF set-profile o+e> ($env.TMPDIR | path join c.full) }
try { ^$env.RUST set-profile o+e> ($env.TMPDIR | path join r.full) }
try { ^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected) }
try { ^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual) }
compare "spa-acp-tool/set-profile-no-args"
