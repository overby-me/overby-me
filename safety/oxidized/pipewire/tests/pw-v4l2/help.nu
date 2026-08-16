source ../helpers.nu

# The bash original read `$?` after `cmd ... || true`, so the recorded exit
# status is always 0 (|| true resets $? before it is read). Reproduce that:
# append a literal `exit=0` to both outputs.
try { ^$env.REF -h o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -h o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-v4l2 -h"
