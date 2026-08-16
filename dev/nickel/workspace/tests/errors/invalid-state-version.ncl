# Error snapshot test: invalid state version format
#
# The state-version field must match "YY.MM" format (e.g. "25.05").
# This file should fail with a contract violation when evaluated.
#
# Run: nickel export tests/errors/invalid-state-version.ncl
# Expected: contract error about invalid state version "unstable"
# Expected hint: State version must match the pattern "YY.MM"
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

{
  name = "error-test",
  machines = {
    bad-machine = {
      system = "x86_64-linux",
      state-version = "unstable",
    },
  },
} | WorkspaceConfig
