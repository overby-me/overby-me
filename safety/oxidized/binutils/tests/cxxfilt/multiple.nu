source ../helpers.nu

# Test c++filt with multiple mangled symbols
let input = $env.TMPDIR | path join input
"_Z3fooi
_Z3barv
_ZN3Foo3bazEi
not_mangled
_ZNSt6vectorIiSaIiEE9push_backERKi
" | save -f --raw $input
try { open --raw $input | ^$env.REF o+e> ($env.TMPDIR | path join expected) }
try { open --raw $input | ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
compare "c++filt multiple symbols"
