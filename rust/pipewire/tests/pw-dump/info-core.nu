source ../helpers.nu

# Core info block: cookie, user-name, host-name, version, name, change-mask, props.
# Cookie and hostname vary per run/host so we normalize them.
let cfull = $env.TMPDIR | path join c.full
let rfull = $env.TMPDIR | path join r.full
^$env.REF 0 o+e> $cfull
^$env.RUST 0 o+e> $rfull
# Normalize cookie (uint32) and host-name/user-name (vary by host).
let sedprog = 's|"cookie": [0-9]+|"cookie": COOKIE|; s|"user-name": "[^"]*"|"user-name": "USER"|; s|"host-name": "[^"]*"|"host-name": "HOST"|'
^sed -E $sedprog $cfull o> ($env.TMPDIR | path join expected)
^sed -E $sedprog $rfull o> ($env.TMPDIR | path join actual)
compare "pw-dump 0 (Core info block: cookie/user-name/host-name/version/name/props)"
