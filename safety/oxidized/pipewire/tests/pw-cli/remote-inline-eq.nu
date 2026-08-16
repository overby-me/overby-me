source ../helpers.nu

try { ^$env.REF --remote=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --remote=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli --remote=foo (REPL connect-attempt)"
