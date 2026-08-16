source ../helpers.nu

try { ^$env.REF -hX o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hX o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump -hX (-h short-circuits to help, X never seen)"
