source ../helpers.nu

# Test c++filt -t / --types: also demangle type encodings
let in_file = $env.TMPDIR | path join in
"Pi\n" | save -f --raw $in_file
try { open --raw $in_file | ^$env.REF -t o+e> ($env.TMPDIR | path join expected) }
try { open --raw $in_file | ^$env.RUST -t o+e> ($env.TMPDIR | path join actual) }
compare "c++filt -t"
