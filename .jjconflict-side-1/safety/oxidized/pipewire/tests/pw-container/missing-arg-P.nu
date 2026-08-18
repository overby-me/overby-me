source ../helpers.nu

# pw-container -P requires an argument (properties string).
try { ^$env.REF -P o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -P o+e> ($env.TMPDIR | path join actual) }
compare "pw-container -P (missing properties argument)"
