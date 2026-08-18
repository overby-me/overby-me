# Step contract for tangled workflows
#
# A step is a single unit of execution within a workflow.
# Steps execute serially in the order they are defined.
#
# The command field accepts either "run" or "command" as the key.
# "run" is the canonical key; "command" is accepted as an alias.
{
  StepConfig =
    std.contract.from_validator (fun value =>
      if !(std.is_record value) then
        'Error { message = "expected a record, got %{std.typeof value}" }
      else if !std.record.has_field "name" value then
        'Error { message = "step is missing required field \"name\"" }
      else if !(std.is_string value.name) then
        'Error { message = "step \"name\" must be a string, got %{std.typeof value.name}" }
      else
        let has_run = std.record.has_field "run" value in
        let has_command = std.record.has_field "command" value in
        if !has_run && !has_command then
          'Error {
            message = "step \"%{value.name}\" is missing a command",
            notes = ["Provide either \"run\" or \"command\" field"],
          }
        else if has_run && !(std.is_string value.run) then
          'Error { message = "step \"%{value.name}\": \"run\" must be a string" }
        else if has_command && !(std.is_string value.command) then
          'Error { message = "step \"%{value.name}\": \"command\" must be a string" }
        else
          'Ok
    ),
}
