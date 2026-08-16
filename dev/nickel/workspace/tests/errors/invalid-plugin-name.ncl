# Error test: invalid plugin name (starts with a number)
#
# This should fail PluginConfig validation because plugin names
# must match the Name contract: [a-zA-Z_][a-zA-Z0-9_-]*
let { PluginConfig, .. } = import "../../contracts/plugin.ncl" in

{
  name = "123-bad-name",
} | PluginConfig
