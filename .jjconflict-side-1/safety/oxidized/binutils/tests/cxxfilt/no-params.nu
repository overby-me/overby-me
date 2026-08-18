source ../helpers.nu

# Test c++filt -p / --no-params: drop the function-arguments part
let in_file = $env.TMPDIR | path join in
"_Z3fooPi\n" | save -f --raw $in_file
try { open --raw $in_file | ^$env.REF -p o+e> ($env.TMPDIR | path join expected) }
try { open --raw $in_file | ^$env.RUST -p o+e> ($env.TMPDIR | path join actual) }
compare "c++filt -p"
