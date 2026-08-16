source ../helpers.nu

let conf = ($env.SRC | path join src/daemon/filter-chain/sink-dolby-pro-logic-ii.conf)
^$env.REF $conf o+e> ($env.TMPDIR | path join expected)
^$env.RUST $conf o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/conf-fc-sink-dolby-pro-logic-ii"
