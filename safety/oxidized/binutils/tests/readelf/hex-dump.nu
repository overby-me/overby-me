source ../helpers.nu

# Test readelf --hex-dump=.shstrtab: hex dump of a named section
try { ^$env.REF --hex-dump=.shstrtab $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --hex-dump=.shstrtab $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf --hex-dump"
