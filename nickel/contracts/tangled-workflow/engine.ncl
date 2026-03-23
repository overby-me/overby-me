# Engine contract for tangled workflows
#
# The engine determines how dependencies are resolved and how steps
# are executed. Currently "nix" and "nixery" are accepted (case-insensitive).
{
  Engine
    | doc "Execution engine for the workflow"
    =
      std.contract.from_validator (fun value =>
        let valid_engines = ["nix", "nixery"] in
        if !(std.is_string value) then
          'Error { message = "expected a string, got %{std.typeof value}" }
        else if std.array.elem (std.string.lowercase value) valid_engines then
          'Ok
        else
          'Error {
            message = "unknown engine \"%{value}\"",
            notes = [
              "Supported engines: %{std.string.join ", " valid_engines}",
            ],
          }
      ),
}
