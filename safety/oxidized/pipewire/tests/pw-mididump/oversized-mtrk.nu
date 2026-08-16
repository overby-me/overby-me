source ../helpers.nu

# MTrk size header says 11 bytes but body is actually 12 bytes. C reads
# past the declared size to find EOT; we should match that tolerance.
let mid = $env.TMPDIR | path join timesig.mid
0x[4d546864 00000006 00000001 0060 4d54726b 0000000b 00 ff58040402180800 ff2f00] | save -f --raw $mid
# The bash original used `|| true`, so $? is always 0.
try { ^$env.REF $mid o+e> ($env.TMPDIR | path join expected) }
let e_ref = 0
try { ^$env.RUST $mid o+e> ($env.TMPDIR | path join actual) }
let e_rust = 0
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-mididump MTrk size off-by-one (read past declared size)"
