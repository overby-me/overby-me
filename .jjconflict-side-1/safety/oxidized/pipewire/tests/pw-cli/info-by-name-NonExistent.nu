source ../helpers.nu

try { ^$env.REF info NonExistent o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info NonExistent o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info NonExistent (by-name lookup miss)"
