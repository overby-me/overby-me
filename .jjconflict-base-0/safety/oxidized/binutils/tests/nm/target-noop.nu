source ../helpers.nu

# Test nm --target=NAME: should be silently accepted (no BFD targets)
try { ^$env.REF --target=elf64-x86-64 $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --target=elf64-x86-64 $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --target=elf64-x86-64"
