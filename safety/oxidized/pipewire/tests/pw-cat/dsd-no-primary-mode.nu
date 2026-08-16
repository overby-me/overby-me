source ../helpers.nu

try { ^$env.REF --dsd o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --dsd o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --dsd alone (sub-mode requires --playback or --record)"
