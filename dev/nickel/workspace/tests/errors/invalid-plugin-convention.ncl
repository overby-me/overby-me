# Error test: invalid plugin convention — missing required field `path`
#
# The PluginConvention contract requires `path`, `output`, and `builder`.
# This test verifies that omitting `path` is rejected.
#
# Expected: contract violation (missing field `path`)
let { PluginConfig, .. } = import "../../contracts/plugin.ncl" in

({
  name = "bad-plugin",
  conventions = {
    broken = {
      # path is missing — should fail
      output = "packages",
      builder = "rust",
    },
  },
}
) | PluginConfig
