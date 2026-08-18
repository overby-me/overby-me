source ../helpers.nu

# Test objdump -d --no-show-raw-insn: disassembly with the bytes column suppressed
try { ^$env.REF -d --no-show-raw-insn $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d --no-show-raw-insn $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -d --no-show-raw-insn"
