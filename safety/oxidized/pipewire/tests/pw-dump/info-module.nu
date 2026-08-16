source ../helpers.nu

# pw-dump <module-id> should emit the per-class "info" block:
# Module's info has name, filename, args, change-mask flags, and a
# sorted props dict.
let expected = $env.TMPDIR | path join expected
let actual = $env.TMPDIR | path join actual
^$env.REF 1 o> $expected e> ($env.TMPDIR | path join c.err)
^$env.RUST 1 o> $actual e> ($env.TMPDIR | path join r.err)
# Normalize the absolute module path under /nix/store (varies per
# pipewire derivation revision).
^sed -i -E 's|/nix/store/[a-z0-9]{32}-[^"]*pipewire-[^"]+/lib/pipewire-[^/]+/|NIXLIBPATH/|g' $expected $actual
compare "pw-dump 1 (Module info block: name, filename, args, change-mask, sorted props)"
