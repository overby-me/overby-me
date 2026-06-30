# Engine contract for tangled workflows
#
# The engine determines how dependencies are resolved and how steps
# are executed. Accepted values (case-insensitive):
#   - "microvm": each workflow runs inside its own QEMU microVM — a full
#                (NixOS) machine. See https://blog.tangled.org/spindle-microvm/
#   - "nixery":  each step runs in a Docker container built on-the-fly by
#                Nixery from nixpkgs (upstream Spindle default).
#   - "nix":     native-Nix engine from this repo's `tangled/spindle-nix-engine`
#                runner — steps run as sandboxed child processes, no container.
{
  Engine
    | doc "Execution engine for the workflow"
    =
      std.contract.from_validator (fun value =>
        let valid_engines = ["microvm", "nixery", "nix"] in
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
