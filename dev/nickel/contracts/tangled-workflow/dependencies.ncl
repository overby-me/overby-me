# Dependencies contract for tangled workflows
#
# Two shapes are accepted, depending on the engine:
#
#   - microvm: a flat list of package names made available to every step.
#     This form only applies to NixOS images.
#       dependencies = [ "go", "gcc", "github:nixos/nixpkgs#hello" ]
#
#   - nixery / nix: a map keyed by Nix registry, mapping to lists of package
#     names. The engine resolves these packages and makes them available
#     during execution.
#       dependencies = { nixpkgs = [ "go", "gcc" ], "nixpkgs/nixpkgs-unstable" = [ "bun" ] }
{
  Dependencies
    | doc m%"
      Packages to install.

      microvm engine: a flat array of package names (NixOS images only).
      nixery/nix engines: a map of Nix registry reference (e.g. "nixpkgs",
      "nixpkgs/nixpkgs-unstable") to an array of package attribute names.
    "%
    =
      let is_string_array = fun v => std.is_array v && std.array.all std.is_string v in
      std.contract.from_validator (fun value =>
        if std.is_array value then
          if std.array.all std.is_string value then
            'Ok
          else
            'Error {
              message = "microvm dependencies must be a flat list of package-name strings",
            }
        else if std.is_record value then
          let bad =
            value
            |> std.record.to_array
            |> std.array.filter (fun entry => !(is_string_array entry.value))
          in
          if bad == [] then
            'Ok
          else
            'Error {
              message = "nixery/nix dependencies must map each registry to a list of package-name strings",
              notes =
                std.array.map (fun entry => "registry \"%{entry.field}\" is not a list of strings") bad,
            }
        else
          'Error {
            message = "dependencies must be a list of package names (microvm) or a map of registry to package lists (nixery/nix), got %{std.typeof value}",
          }
      ),
}
