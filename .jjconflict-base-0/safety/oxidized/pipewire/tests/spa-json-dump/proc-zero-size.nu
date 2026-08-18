source ../helpers.nu

try { ^$env.REF /proc/cpuinfo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /proc/cpuinfo o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump /proc/cpuinfo (stat size=0 → mmap EINVAL)"
