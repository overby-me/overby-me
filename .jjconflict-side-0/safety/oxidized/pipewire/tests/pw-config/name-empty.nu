source ../helpers.nu

# pw-config --name= (empty). Same code path as no-suffix: empty string
# doesn't end with .conf so we reject early.
try { ^$env.REF "--name=" o+e> ($env.TMPDIR | path join c.full) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST "--name=" o+e> ($env.TMPDIR | path join r.full) }
let e_rust = $env.LAST_EXIT_CODE
^sed -E 's|\[[0-9:.]+\]|[TIME]|' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected)
^sed -E 's|\[[0-9:.]+\]|[TIME]|' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-config --name= (empty name rejected with 'does not end with .conf')"
