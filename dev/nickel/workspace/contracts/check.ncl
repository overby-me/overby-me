# Check contracts for nix-workspace
#
# Defines the CheckConfig contract for CI check definitions.
# Checks are discovered from the checks/ convention directory
# or declared explicitly in workspace.ncl.
#
# Each check config maps to a checks.<system>.<name> flake output.
# Checks are derivations that succeed (exit 0) when the check passes
# and fail (non-zero exit) when it doesn't. They are run by `nix flake check`.
let { System, NonEmptyString, .. } = import "./common.ncl" in

{
  CheckConfig = {
    description
      | String
      | doc "Human-readable description of what this check verifies"
      | optional,

    command
      | String
      | doc m%"
          Shell command to run as the check body.
          The command runs in a bash shell with the workspace source
          available in the working directory. Exit 0 = pass, non-zero = fail.

          Example: "cargo test --workspace"
        "%
      | optional,

    path
      | String
      | doc m%"
          Explicit path to a .nix file that produces a check derivation,
          relative to the workspace root. When set, `command` is ignored
          and the .nix file is imported directly.
          When auto-discovered from checks/, this is set automatically.
        "%
      | optional,

    packages
      | Array String
      | doc m%"
          Packages (nixpkgs attribute names) to make available in the
          check environment. Only relevant when using `command`.
        "%
      | default
      = [],

    env
      | { _ : String }
      | doc "Environment variables to set during the check run"
      | default
      = {},

    systems
      | Array System
      | doc m%"
          Override the workspace-level systems list for this check.
          When set, the check is only run on the specified systems.
        "%
      | optional,

    timeout
      | std.contract.from_validator (fun value =>
        if std.is_number value then
          if value > 0 then
            'Ok
          else
            'Error {
              message = "timeout must be positive, got %{std.to_string value}",
            }
        else
          'Error {
            message = "timeout must be a number (seconds), got %{std.typeof value}",
          }
      )
      | doc m%"
          Maximum time in seconds the check is allowed to run.
          If the check exceeds this duration, it is killed and reported
          as failed. Defaults to 600 (10 minutes).
        "%
      | default
      = 600,

    inputs-from
      | Array String
      | doc m%"
          Other packages whose build inputs should be available
          during the check. Only relevant when using `command`.
        "%
      | default
      = [],

    extra-config
      | { _ : Dyn }
      | doc m%"
          Escape hatch: additional configuration passed through to the
          check builder. Use sparingly.
        "%
      | default
      = {},
  },
}
