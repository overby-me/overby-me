# Common contracts shared across nix-workspace
#
# Provides foundational types: System, Name, and other shared contracts
# used by workspace, package, shell, and machine contracts.
{
  System
    | doc "A valid Nix system triple"
    =
      std.contract.from_validator (fun value =>
        let valid_systems = [
          "x86_64-linux",
          "aarch64-linux",
          "x86_64-darwin",
          "aarch64-darwin",
        ]
        in
        if !(std.is_string value) then
          'Error {
            message = "expected a string, got %{std.typeof value}",
            notes = [
              "System must be one of: %{std.string.join ", " valid_systems}",
            ],
          }
        else if std.array.elem value valid_systems then
          'Ok
        else
          let suggestion =
            valid_systems
            |> std.array.filter (fun s => std.string.contains (std.string.substring 0 4 value) s)
            |> (fun matches =>
              if std.array.length matches > 0 then
                " — did you mean \"%{std.array.first matches}\"?"
              else
                ""
            )
          in
          'Error {
            message = "invalid system \"%{value}\"%{suggestion}",
            notes = [
              "Valid systems: %{std.string.join ", " valid_systems}",
            ],
          }
      ),

  Name
    | doc m%"
      A valid workspace or output name.
      Must start with a letter or underscore, followed by alphanumeric
      characters, hyphens, or underscores. This ensures names are valid
      Nix derivation names.
    "%
    =
      std.contract.from_validator (fun value =>
        if !(std.is_string value) then
          'Error {
            message = "expected a string, got %{std.typeof value}",
          }
        else if std.string.length value == 0 then
          'Error {
            message = "name must not be empty",
          }
        else if std.string.is_match "^[a-zA-Z_][a-zA-Z0-9_-]*$" value then
          'Ok
        else
          'Error {
            message = "invalid name \"%{value}\"",
            notes = [
              "Names must match the pattern [a-zA-Z_][a-zA-Z0-9_-]*",
              "Names must start with a letter or underscore.",
              "Only alphanumeric characters, hyphens, and underscores are allowed.",
            ],
          }
      ),

  NonEmptyString
    | doc "A string that must not be empty"
    =
      std.contract.from_validator (fun value =>
        if !(std.is_string value) then
          'Error { message = "expected a string, got %{std.typeof value}" }
        else if std.string.length value == 0 then
          'Error { message = "string must not be empty" }
        else
          'Ok
      ),

  RelativePath
    | doc m%"
      A relative file path (must not start with /).
      Used for source directories and file references within a workspace.
    "%
    =
      std.contract.from_validator (fun value =>
        if !(std.is_string value) then
          'Error { message = "expected a string, got %{std.typeof value}" }
        else if std.string.is_match "^/" value then
          'Error {
            message = "path must be relative, got \"%{value}\"",
            notes = ["Remove the leading '/' to make this a relative path."],
          }
        else if std.string.length value == 0 then
          'Error { message = "path must not be empty" }
        else
          'Ok
      ),

  ModuleRef
    | doc "A reference to a NixOS or home-manager module by name or path"
    =
      std.contract.from_validator (fun value =>
        if !(std.is_string value) then
          'Error { message = "expected a string (module name or path), got %{std.typeof value}" }
        else if std.string.length value == 0 then
          'Error { message = "module reference must not be empty" }
        else
          'Ok
      ),
}
