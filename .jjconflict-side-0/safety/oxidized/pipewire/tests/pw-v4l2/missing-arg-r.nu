source ../helpers.nu

try { ^$env.REF -r o+e> ($env.TMPDIR | path join c.full) }
try { ^$env.RUST -r o+e> ($env.TMPDIR | path join r.full) }
# Normalize C's full-path argv0 to TOOL.
^sed -E "s#/[^[:space:]:]+/pw-v4l2#TOOL#g" ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected)
^sed -E "s#/[^[:space:]:]+/pw-v4l2#TOOL#g" ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual)
^sed -i "s#^pw-v4l2:#TOOL:#" ($env.TMPDIR | path join actual)
^sed -i "s#^pw-v4l2 -#TOOL -#" ($env.TMPDIR | path join actual)
compare "pw-v4l2 -r (missing arg)"
