source ../helpers.nu

# Track body starts with status byte (no delta-time): C silently
# terminates the track without synthesizing EOT.
let mid = $env.TMPDIR | path join no-delta.mid
0x[4d546864 00000006 00000001 0060 4d54726b 00000003 ff2f00] | save -f --raw $mid
# The bash original used `|| true`, so $? is always 0.
try { ^$env.REF $mid o+e> ($env.TMPDIR | path join expected) }
let e_ref = 0
try { ^$env.RUST $mid o+e> ($env.TMPDIR | path join actual) }
let e_rust = 0
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-mididump no-delta-partial (silent terminate)"
