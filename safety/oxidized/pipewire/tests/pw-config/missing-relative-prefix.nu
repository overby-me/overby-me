source ../helpers.nu

# pw-config -p <relative-path> -n <name>. C's get_config_path tries
# get_abs_path first; for relative prefixes it falls through env/home/
# configdir lookups WITHOUT calling conf_load directly. So the 425 W
# line from conf_load() does NOT appear here (unlike the absolute-path
# case covered by missing-prefix-path.nu).
try { ^$env.REF "--prefix=relpath" "--name=test.conf" o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST "--prefix=relpath" "--name=test.conf" o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
^sed -i -E 's|\[[0-9:.]+\]|[TIME]|g; s|0x[0-9a-f]+|0xPTR|g' ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-config -p relpath (no 425 W line; relative prefix bypasses direct fopen)"
