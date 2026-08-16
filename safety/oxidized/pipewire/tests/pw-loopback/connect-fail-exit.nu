source ../helpers.nu

with-env {PIPEWIRE_REMOTE: "non-existent"} {
    try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
    let e_ref = $env.LAST_EXIT_CODE
    try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
    let e_rust = $env.LAST_EXIT_CODE
    $"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
    $"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
}
^sed -i 's/pw-loopback-[0-9]*/pw-loopback-PID/g' ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-loopback connect-fail exits 255"
