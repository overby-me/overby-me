source ../helpers.nu

try { ^$env.REF /dev/null o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /dev/null o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump /dev/null (empty file → mmap EINVAL)"
