# Error snapshot test: invalid dependency reference
#
# Dependencies must be valid Name strings (DependencyRef = Name).
# Using a number should trigger a contract violation.
#
# Expected: contract error on dependencies.bad value (not a valid Name)
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

(
  {
    name = "test-workspace",
    dependencies = {
      bad = 42,
    },
  } | WorkspaceConfig
)
