source ../helpers.nu

try { ^$env.REF -q-1 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -q-1 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -q-1 (negative quality → 'bad quality -1')"
