source ../helpers.nu

# Test c++filt demangling of C++ symbols
try { "_Z3fooi\n" | ^$env.REF o+e> ($env.TMPDIR | path join expected) }
try { "_Z3fooi\n" | ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
compare "c++filt basic demangling"
