source ../helpers.nu

# `help` as a subcommand (vs --help flag) lists the spa-acp-tool
# interactive command set. Same output as the bare REPL prompt's
# help, exercised here once at startup.
try { ^$env.REF help o+e> ($env.TMPDIR | path join c.full) }
try { ^$env.RUST help o+e> ($env.TMPDIR | path join r.full) }
try { ^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected) }
try { ^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual) }
compare "spa-acp-tool help (subcommand prints interactive command list)"
