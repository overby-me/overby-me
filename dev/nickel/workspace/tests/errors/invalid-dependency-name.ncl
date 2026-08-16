# Error snapshot test: invalid dependency name
#
# DependencyRef is an alias for Name, which requires names to start
# with a letter or underscore and contain only alphanumeric characters,
# hyphens, or underscores.
#
# A dependency target that starts with a number should trigger a
# contract violation from the Name validator.
#
# Expected: contract error on dependencies.bad — invalid name "3invalid"
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

(
  {
    name = "test-workspace",
    dependencies = {
      bad = "3invalid",
    },
  } | WorkspaceConfig
)
