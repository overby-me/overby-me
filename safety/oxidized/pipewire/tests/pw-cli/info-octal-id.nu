source ../helpers.nu

# strtoul base 0 with leading `0` means octal. `info 010` = id 8.
try { ^$env.REF "info 010" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "info 010" o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info 010 (octal id parsing)"
