source ../helpers.nu

# Test c++filt -n / --no-strip-underscore: don't strip leading underscores
let in_file = $env.TMPDIR | path join in
"_Z1av\n" | save -f --raw $in_file
try { open --raw $in_file | ^$env.REF -n o+e> ($env.TMPDIR | path join expected) }
try { open --raw $in_file | ^$env.RUST -n o+e> ($env.TMPDIR | path join actual) }
compare "c++filt -n"
