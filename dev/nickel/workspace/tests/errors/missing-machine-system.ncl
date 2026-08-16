# Error snapshot test: missing required system field in MachineConfig
#
# The `system` field is required by MachineConfig.
# This file should fail with a contract violation when evaluated.
#
# Run: nickel export tests/errors/missing-machine-system.ncl
# Expected: contract error about missing field `system`
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

{
  name = "error-test",
  machines = {
    my-machine = {
      state-version = "25.05",
      modules = ["desktop"],
    },
  },
} | WorkspaceConfig
