source ../helpers.nu

try { ^$env.REF -r44100 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r44100 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -r44100 (attached short value parses)"
