source ../helpers.nu

# `pw-cli '#comment'` is treated by C's parse() as a comment line:
# strchr('#') truncates the buffer to "", then the empty-input no-op
# branch fires and the command runs silently with exit 0.
try { ^$env.REF "#comment" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "#comment" o+e> ($env.TMPDIR | path join actual) }
# bash read $? after `|| true`, so the recorded exit code is always 0.
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cli '#comment' (entire input is a comment, silent no-op)"
