source ../helpers.nu

try { ^$env.REF --help=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --help=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli/help-with-arg"
