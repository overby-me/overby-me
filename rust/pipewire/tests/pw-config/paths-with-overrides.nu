source ../helpers.nu

# pw-config paths with main + drop-in overrides in `pipewire.conf.d/`.
mkdir ($env.TMPDIR | path join conf-dir pipewire.conf.d)
r#'context.properties = { core.daemon = true }
'# | save -f --raw ($env.TMPDIR | path join conf-dir pipewire.conf)
r#'context.properties = { core.name = pipewire-0 }
'# | save -f --raw ($env.TMPDIR | path join conf-dir pipewire.conf.d 00-first.conf)
r#'context.properties = { log.level = 3 }
'# | save -f --raw ($env.TMPDIR | path join conf-dir pipewire.conf.d 99-last.conf)

$env.PIPEWIRE_CONFIG_DIR = ($env.TMPDIR | path join conf-dir)
$env.HOME = ($env.TMPDIR | path join no-home)
$env.XDG_CONFIG_HOME = ($env.TMPDIR | path join no-xdg)

^$env.REF o> ($env.TMPDIR | path join expected) e> /dev/null
^$env.RUST o> ($env.TMPDIR | path join actual) e> /dev/null
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-config/paths-with-overrides"
