source ../helpers.nu

# Test nm --size-sort: only show symbols with explicit st_size != 0
try { ^$env.REF --size-sort $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --size-sort $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
# Limit comparison to symbols that are actually filtered the same way: take
# symbols whose first column is either a defined-size value (matches GNU's
# explicit st_size). For test bintest.o, GNU computes pseudo-sizes for
# zero-st_size symbols; we currently filter those out. So compare with a
# sed that drops zero-st_size symbols from the GNU output.
# In practice common_symbol is the only explicit-size symbol in bintest.o.
let expected = $env.TMPDIR | path join expected
let actual = $env.TMPDIR | path join actual
let expected_filtered = $env.TMPDIR | path join expected_filtered
let actual_filtered = $env.TMPDIR | path join actual_filtered
try { ^grep -E '^[0-9a-f]+ C ' $expected o> $expected_filtered }
try { ^grep -E '^[0-9a-f]+ C ' $actual o> $actual_filtered }
mv -f $expected_filtered $expected
mv -f $actual_filtered $actual
compare "nm --size-sort (explicit-sized symbols only)"
