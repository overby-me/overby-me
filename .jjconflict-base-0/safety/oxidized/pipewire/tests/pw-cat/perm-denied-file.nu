source ../helpers.nu

# Create a file we can't read
let locked = $env.TMPDIR | path join locked.wav
touch $locked
^chmod 000 $locked
try { ^$env.REF -p $locked o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p $locked o+e> ($env.TMPDIR | path join actual) }
^chmod 644 $locked  # cleanup
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cat -p inaccessible-file (sndfile permission-denied)"
