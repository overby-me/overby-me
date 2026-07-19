source ../helpers.nu

# pw-container -r requires an argument (remote daemon name).
try { ^$env.REF -r o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r o+e> ($env.TMPDIR | path join actual) }
compare "pw-container -r (missing remote argument)"
