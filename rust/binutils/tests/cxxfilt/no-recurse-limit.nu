source ../helpers.nu

# Test c++filt --no-recurse-limit: accept the demangler knob
let in_file = $env.TMPDIR | path join in
"_Z3fooP1A\n" | save -f --raw $in_file
try { open --raw $in_file | ^$env.REF --no-recurse-limit o+e> ($env.TMPDIR | path join expected) }
try { open --raw $in_file | ^$env.RUST --no-recurse-limit o+e> ($env.TMPDIR | path join actual) }
compare "c++filt --no-recurse-limit"
