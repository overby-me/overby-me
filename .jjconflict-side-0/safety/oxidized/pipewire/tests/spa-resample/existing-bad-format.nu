source ../helpers.nu

"not audio\n" | save -f --raw ($env.TMPDIR | path join notaudio.wav)
try { ^$env.REF ($env.TMPDIR | path join notaudio.wav) /tmp/out.wav o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST ($env.TMPDIR | path join notaudio.wav) /tmp/out.wav o+e> ($env.TMPDIR | path join actual) }
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "spa-resample existing-bad-format (Format not recognised)"
