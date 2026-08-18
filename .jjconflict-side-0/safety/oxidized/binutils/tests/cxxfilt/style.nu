source ../helpers.nu

# Test c++filt -s gnu-v3: accept the style flag (Itanium ABI)
let in_file = $env.TMPDIR | path join in
"_Z3fooP1A\n" | save -f --raw $in_file
try { open --raw $in_file | ^$env.REF -s gnu-v3 o+e> ($env.TMPDIR | path join expected) }
try { open --raw $in_file | ^$env.RUST -s gnu-v3 o+e> ($env.TMPDIR | path join actual) }
compare "c++filt -s gnu-v3"
