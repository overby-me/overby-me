source ../helpers.nu

try { ^$env.REF -pfoo bar baz o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -pfoo bar baz o+e> ($env.TMPDIR | path join actual) }
compare "pw-link -pfoo bar baz (attached value)"
