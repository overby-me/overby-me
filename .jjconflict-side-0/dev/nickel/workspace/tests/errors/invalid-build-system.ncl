# Error snapshot test: invalid build-system value
#
# Expected: contract violation pointing at "python" with a hint
# about valid build systems ("rust", "go", "generic").
#
# Run: nickel export tests/errors/invalid-build-system.ncl
# Expected exit code: 1
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

{
  name = "error-test",
  packages = {
    bad-pkg = {
      build-system = "python",
    },
  },
} | WorkspaceConfig
