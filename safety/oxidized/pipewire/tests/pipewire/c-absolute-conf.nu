source ../helpers.nu

# pipewire -c <absolute-path>: C's pw_conf_load_conf takes the
# get_abs_path branch, calls conf_load() directly (which emits the
# 425 W line on open failure) BEFORE the 1182 try_load_conf log line.
# Relative paths skip conf_load() and don't emit 425.
try { ^$env.REF -c /nonexistent.conf o+e> ($env.TMPDIR | path join c.full) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST -c /nonexistent.conf o+e> ($env.TMPDIR | path join r.full) }
let e_rust = $env.LAST_EXIT_CODE
^sed -E 's|\[[0-9:.]+\]|[TIME]|; s|0x[0-9a-f]+|0xPTR|g' ($env.TMPDIR | path join c.full) o> ($env.TMPDIR | path join expected)
^sed -E 's|\[[0-9:.]+\]|[TIME]|; s|0x[0-9a-f]+|0xPTR|g' ($env.TMPDIR | path join r.full) o> ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pipewire -c /nonexistent.conf (absolute → emits 425 W line)"
