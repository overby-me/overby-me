# Error snapshot test: missing required field
#
# The `name` field is required by WorkspaceConfig.
# This file should fail with a contract violation when evaluated.
#
# Run: nickel export tests/errors/missing-field.ncl
# Expected: contract error about missing field `name`
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

{
  description = "A workspace without a name",
  systems = ["x86_64-linux"],
} | WorkspaceConfig
