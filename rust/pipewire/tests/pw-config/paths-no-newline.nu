source ../helpers.nu

# pw-config -L paths: omit newlines.
mkdir ($env.TMPDIR | path join conf-dir)
r#'context.properties = { core.daemon = true }
'# | save -f --raw ($env.TMPDIR | path join conf-dir pipewire.conf)

$env.PIPEWIRE_CONFIG_DIR = ($env.TMPDIR | path join conf-dir)
$env.HOME = ($env.TMPDIR | path join no-home)
$env.XDG_CONFIG_HOME = ($env.TMPDIR | path join no-xdg)

^$env.REF -L paths o> ($env.TMPDIR | path join expected) e> /dev/null
^$env.RUST -L paths o> ($env.TMPDIR | path join actual) e> /dev/null
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-config/paths-no-newline"
