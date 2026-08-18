source ../helpers.nu

# pw-config -n custom.conf paths
mkdir ($env.TMPDIR | path join conf-dir)
r#'jack.properties = { foo = bar }
'# | save -f --raw ($env.TMPDIR | path join conf-dir jack.conf)

$env.PIPEWIRE_CONFIG_DIR = ($env.TMPDIR | path join conf-dir)
$env.HOME = ($env.TMPDIR | path join no-home)
$env.XDG_CONFIG_HOME = ($env.TMPDIR | path join no-xdg)

^$env.REF -n jack.conf paths o> ($env.TMPDIR | path join expected) e> /dev/null
^$env.RUST -n jack.conf paths o> ($env.TMPDIR | path join actual) e> /dev/null
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
compare "pw-config/paths-custom-name"
