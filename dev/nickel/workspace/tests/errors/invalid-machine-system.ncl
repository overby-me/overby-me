# Error snapshot test: invalid machine system value
#
# A machine config with an invalid system string should fail
# contract validation with a helpful error message.
#
# Expected error: NW0xx contract violation on `system` field
# Expected hint: about valid system values
#
# Run: nickel export tests/errors/invalid-machine-system.ncl
# Expected exit code: 1
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

{
  name = "error-test",
  machines = {
    bad-machine = {
      system = "x86-linux",
      state-version = "25.05",
    },
  },
} | WorkspaceConfig
