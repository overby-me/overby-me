source ../helpers.nu

# Test c++filt --strip-underscore: strip leading underscore before demangling, restore on failure
let input = $env.TMPDIR | path join input
"_Z1av
__Z1av
_main
main
" | save -f --raw $input
try { open --raw $input | ^$env.REF --strip-underscore o+e> ($env.TMPDIR | path join expected) }
try { open --raw $input | ^$env.RUST --strip-underscore o+e> ($env.TMPDIR | path join actual) }
compare "c++filt --strip-underscore"
