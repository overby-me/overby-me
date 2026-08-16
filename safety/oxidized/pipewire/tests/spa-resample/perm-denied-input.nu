source ../helpers.nu

touch ($env.TMPDIR | path join locked.wav)
^chmod 000 ($env.TMPDIR | path join locked.wav)
try { ^$env.REF ($env.TMPDIR | path join locked.wav) ($env.TMPDIR | path join out.wav) o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST ($env.TMPDIR | path join locked.wav) ($env.TMPDIR | path join out.wav) o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
^chmod 644 ($env.TMPDIR | path join locked.wav)  # cleanup
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample inaccessible-input (Permission denied)"
