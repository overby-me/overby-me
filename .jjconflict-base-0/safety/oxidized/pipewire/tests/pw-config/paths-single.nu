source ../helpers.nu

# pw-config paths with a single config file (no overrides).
mkdir ($env.TMPDIR | path join conf-dir)
r#'context.properties = {
    core.daemon = true
    core.name = pipewire-0
}
'# | save -f --raw ($env.TMPDIR | path join conf-dir pipewire.conf)

# Force the C tool to only see PIPEWIRE_CONFIG_DIR; HOME/XDG_CONFIG_HOME
# defaults would otherwise interfere.
$env.PIPEWIRE_CONFIG_DIR = ($env.TMPDIR | path join conf-dir)
$env.HOME = ($env.TMPDIR | path join no-home)
$env.XDG_CONFIG_HOME = ($env.TMPDIR | path join no-xdg)

^$env.REF o> ($env.TMPDIR | path join expected) e> /dev/null
^$env.RUST o> ($env.TMPDIR | path join actual) e> /dev/null
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-config/paths-single"
