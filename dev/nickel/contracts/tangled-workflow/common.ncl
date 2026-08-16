# Common contracts shared across tangled-workflow
{
  StringOrArray
    | doc m%"
      A value that accepts either a single string or an array of strings.
      When a single string is provided, it is treated as a one-element list.
      The inner contract is applied to each element.
    "%
    = fun inner =>
      std.contract.custom (fun label value =>
        if std.is_string value then
          'Ok (value | inner)
        else if std.is_array value then
          'Ok (value |> std.array.map (fun elem => elem | inner))
        else
          'Error {
            message = "expected a string or array, got %{std.typeof value}",
          }
      ),
}
