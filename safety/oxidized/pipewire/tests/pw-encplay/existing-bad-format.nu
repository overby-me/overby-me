source ../helpers.nu

"not media\n" | save --force --raw ($env.TMPDIR | path join notaudio.txt)
# The bash original used `|| true`, which makes $? always 0, so both exit
# lines below are always exit=0.
try { ^$env.REF ($env.TMPDIR | path join notaudio.txt) o+e> ($env.TMPDIR | path join expected) }
let e_ref = 0
try { ^$env.RUST ($env.TMPDIR | path join notaudio.txt) o+e> ($env.TMPDIR | path join actual) }
let e_rust = 0
^sed -i $"s|($env.TMPDIR)|TMPDIR|g; s|0x[0-9a-fA-F]*|0xPTR|g; s|Statistics: [0-9]* bytes read|Statistics: BYTES bytes read|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save --append --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --append --raw ($env.TMPDIR | path join actual)
compare "pw-encplay existing-bad-format (avformat: Invalid data)"
