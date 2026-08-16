source ../helpers.nu

# Test objdump -d -M intel-mnemonic: mnemonic-only knob keeps AT&T operand syntax
try { ^$env.REF -d -M intel-mnemonic $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d -M intel-mnemonic $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -d -M intel-mnemonic"
