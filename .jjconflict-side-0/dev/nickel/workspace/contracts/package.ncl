# Package contracts for nix-workspace
#
# Defines the shape of package configuration entries discovered
# from the packages/ convention directory or declared in workspace.ncl.
let { System, .. } = import "./common.ncl" in

{
  PackageConfig = {
    src
      | String
      | doc "Source directory relative to the workspace root"
      | optional,

    build-system
      | std.contract.from_validator (fun value =>
        if std.is_string value then
          if std.array.elem value ["rust", "go", "generic"] then
            'Ok
          else
            'Error {
              message = "unknown build-system \"%{value}\"",
              notes = [
                "Supported build systems: \"rust\", \"go\", \"generic\".",
                "Omit build-system to use generic (stdenv.mkDerivation).",
              ],
            }
        else
          'Error {
            message = "build-system must be a string, got %{std.typeof value}",
          }
      )
      | doc "Build system to use for this package"
      | default
      = "generic",

    description
      | String
      | doc "Human-readable description of the package"
      | optional,

    systems
      | Array System
      | doc m%"
          Override the workspace-level systems list for this package.
          When set, the package is only built for the specified systems.
        "%
      | optional,

    build-inputs
      | Array String
      | doc "Runtime dependencies (nixpkgs attribute names)"
      | default
      = [],

    native-build-inputs
      | Array String
      | doc "Build-time dependencies (nixpkgs attribute names)"
      | default
      = [],

    env
      | { _ : String }
      | doc "Environment variables to set during the build"
      | default
      = {},

    meta
      | {
        homepage | String | optional,
        license | String | optional,
        maintainers | Array String | optional,
        platforms | Array String | optional,
        ..
      }
      | doc "Package metadata passed through to the Nix derivation"
      | default
      = {},

    override
      | { _ : Dyn }
      | doc m%"
          Escape hatch: arbitrary key-value pairs forwarded verbatim
          to the underlying Nix builder. Use sparingly.
        "%
      | default
      = {},
  },
}
