source ../helpers.nu

# Test readelf -p .shstrtab: string dump of section header string table
try { ^$env.REF -p .shstrtab $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p .shstrtab $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -p .shstrtab"
