source ../helpers.nu

# pw-config --name=foo (no .conf suffix). C's pw_conf_load_conf_for_context
# (conf.c:1210) rejects names that don't end in .conf with
# "config.name '<name>' does not end with .conf" + Invalid argument
# error + exit code 234 (-EINVAL truncated to u8).
try { ^$env.REF "--name=foo" o+e> ($env.TMPDIR | path join c.full) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST "--name=foo" o+e> ($env.TMPDIR | path join r.full) }
let e_rust = $env.LAST_EXIT_CODE
# Normalize timestamps.
^sed -E 's|\[[0-9:.]+\]|[TIME]|' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected)
^sed -E 's|\[[0-9:.]+\]|[TIME]|' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-config --name=foo (rejected: missing .conf suffix → exit 234)"
