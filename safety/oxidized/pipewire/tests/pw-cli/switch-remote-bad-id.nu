source ../helpers.nu

# Same as disconnect but via switch-remote. Both share find_var
# lookup semantics.
try { ^$env.REF "switch-remote 99" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "switch-remote 99" o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli switch-remote 99 (Error: 'Remote 99 does not exist')"
