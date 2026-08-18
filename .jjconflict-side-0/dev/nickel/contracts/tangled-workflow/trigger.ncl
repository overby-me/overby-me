# Trigger constraint contracts for tangled workflows
#
# Defines the `when` clause: a list of constraints that determine
# which events (push, pull_request, manual) trigger the workflow,
# optionally filtered by branch or tag glob patterns.
let { StringOrArray, .. } = import "./common.ncl" in

let Event
  | doc "A valid trigger event type"
  =
    std.contract.from_validator (fun value =>
      let valid_events = ["push", "pull_request", "manual"] in
      if !(std.is_string value) then
        'Error { message = "expected a string, got %{std.typeof value}" }
      else if std.array.elem value valid_events then
        'Ok
      else
        'Error {
          message = "invalid event \"%{value}\"",
          notes = [
            "Valid events: %{std.string.join ", " valid_events}",
          ],
        }
    )
in

{
  Constraint = {
    event
      | StringOrArray Event
      | doc m%"
        Event type(s) that trigger this workflow.
        Can be a single string or a list: "push", "pull_request", "manual".
      "%,

    branch
      | StringOrArray String
      | doc m%"
        Branch glob pattern(s) to match (e.g. "main", "release/*").
        Required for pull_request events.
        For push events, either branch or tag must be set.
      "%
      | optional,

    tag
      | StringOrArray String
      | doc m%"
        Tag glob pattern(s) to match (e.g. "v*", "release-*").
        Only valid for push events.
      "%
      | optional,
  },
}
