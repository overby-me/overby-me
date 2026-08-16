# Error test: invalid system value
#
# This file is expected to FAIL contract validation.
# Used for error message snapshot testing.
#
# Expected error: NW0xx contract violation on `systems` field
# Expected hint: "did you mean x86_64-linux?"
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

{
  name = "error-test",
  systems = ["x86-linux"],
} | WorkspaceConfig
