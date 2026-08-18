source ../helpers.nu

try { ^$env.REF -ffoo - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -ffoo - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -ffoo (invalid format → 'bad format foo')"
