source ../helpers.nu

# spa-acp-tool doesn't declare --version. Long-form `--version` falls
# through to the getopt "unrecognized option" path.
try { ^$env.REF --version o+e> ($env.TMPDIR | path join c.full) }
try { ^$env.RUST --version o+e> ($env.TMPDIR | path join r.full) }
try { ^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected) }
try { ^grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual) }
compare "spa-acp-tool --version (unrecognized: not declared)"
