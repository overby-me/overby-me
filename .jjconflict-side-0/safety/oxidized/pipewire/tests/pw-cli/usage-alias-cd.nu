source ../helpers.nu

try { ^$env.REF cd o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST cd o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli cd (alias usage)"
