# Overlay contracts for nix-workspace
#
# Defines the OverlayConfig contract for nixpkgs overlay definitions.
# Overlays are discovered from the overlays/ convention directory
# or declared explicitly in workspace.ncl.
#
# Each overlay config maps to an overlays.<name> flake output.
# Overlays are functions (final: prev: { ... }) that extend or
# override packages in nixpkgs.
let { NonEmptyString, RelativePath, .. } = import "./common.ncl" in

{
  OverlayConfig = {
    description
      | String
      | doc "Human-readable description of what this overlay provides"
      | optional,

    path
      | String
      | doc m%"
          Explicit path to the .nix overlay file, relative to the workspace root.
          The file must export a function `final: prev: { ... }`.
          When auto-discovered from overlays/, this is set automatically.
        "%
      | optional,

    priority
      | std.contract.from_validator (fun value =>
        if std.is_number value then
          if value >= 0 then
            'Ok
          else
            'Error {
              message = "priority must be non-negative, got %{std.to_string value}",
            }
        else
          'Error {
            message = "priority must be a number, got %{std.typeof value}",
          }
      )
      | doc m%"
          Overlay application priority (lower = applied first).
          Overlays are applied in priority order when multiple overlays
          are present. Overlays with the same priority are applied in
          alphabetical name order. Defaults to 100.
        "%
      | default
      = 100,

    packages
      | Array String
      | doc m%"
          List of package attribute names this overlay provides or modifies.
          This is informational metadata — it does not affect overlay behavior.
          Useful for documentation and tooling introspection.
        "%
      | default
      = [],

    extra-config
      | { _ : Dyn }
      | doc m%"
          Escape hatch: additional configuration passed through to the
          overlay builder. Use sparingly.
        "%
      | default
      = {},
  },
}
