source ../helpers.nu

# MTrk size includes the SMPTE event + start of EOT marker (just 0xFF),
# leaving an incomplete meta event at the end. C tolerates this and
# synthesizes EOT; we must match that behavior.
let mid = $env.TMPDIR | path join smpte.mid
0x[4d546864 00000006 00000001 0060 4d54726b 0000000b 00 ff54050102030405 00 ff2f00] | save -f --raw $mid
# The bash original used `|| true`, so $? is always 0.
try { ^$env.REF $mid o+e> ($env.TMPDIR | path join expected) }
let e_ref = 0
try { ^$env.RUST $mid o+e> ($env.TMPDIR | path join actual) }
let e_rust = 0
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-mididump SMPTE offset + partial EOT (tolerant)"
