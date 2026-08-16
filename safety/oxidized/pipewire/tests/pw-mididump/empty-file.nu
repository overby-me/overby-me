source ../helpers.nu

"\n" | save -f --raw ($env.TMPDIR | path join empty.mid)
try { ^$env.REF ($env.TMPDIR | path join empty.mid) o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST ($env.TMPDIR | path join empty.mid) o+e> ($env.TMPDIR | path join actual) }
# Normalize the temp file path
^sed -i $"s#($env.TMPDIR)/empty.mid#TMPFILE#g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-mididump empty.mid (Invalid argument)"
