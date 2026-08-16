# Unit tests for common contracts (System, Name, NonEmptyString, RelativePath, ModuleRef)
#
# Run with: nickel eval tests/unit/common.ncl
# All tests return a success message or throw on failure.
#
# Since Nickel doesn't expose `std.contract.is_valid`, we use a custom
# wrapper that catches immediate contract errors via `std.contract.check`
# and returns a boolean.
let { System, Name, NonEmptyString, RelativePath, ModuleRef, .. } = import "../../contracts/common.ncl" in

# ── Helpers ───────────────────────────────────────────────────────
let passes = fun contract value =>
  let wrapper =
    std.contract.custom (fun label v =>
      std.contract.check contract label v
      |> match {
        'Ok _ => 'Ok true,
        'Error _ => 'Ok false,
      }
    )
  in
  (value | wrapper)
in

let fails = fun contract value =>
  !(passes contract value)
in

# ── System ────────────────────────────────────────────────────────
let system_tests = {
  valid_x86_64_linux = passes System "x86_64-linux",
  valid_aarch64_linux = passes System "aarch64-linux",
  valid_x86_64_darwin = passes System "x86_64-darwin",
  valid_aarch64_darwin = passes System "aarch64-darwin",

  rejects_empty = fails System "",
  rejects_typo = fails System "x86-linux",
  rejects_number = fails System 42,
  rejects_bool = fails System true,
  rejects_unknown = fails System "riscv64-linux",
  rejects_null = fails System null,
}
in

# ── Name ──────────────────────────────────────────────────────────
let name_tests = {
  valid_simple = passes Name "hello",
  valid_with_hyphen = passes Name "my-tool",
  valid_with_underscore = passes Name "_private",
  valid_mixed = passes Name "Foo_Bar-123",
  valid_single_char = passes Name "a",
  valid_underscore_only = passes Name "_",

  rejects_empty = fails Name "",
  rejects_starts_number = fails Name "1hello",
  rejects_starts_hyphen = fails Name "-hello",
  rejects_spaces = fails Name "hello world",
  rejects_dots = fails Name "hello.world",
  rejects_slash = fails Name "hello/world",
  rejects_number = fails Name 42,
  rejects_null = fails Name null,
}
in

# ── NonEmptyString ────────────────────────────────────────────────
let non_empty_string_tests = {
  valid_normal = passes NonEmptyString "hello",
  valid_single = passes NonEmptyString "x",
  valid_spaces = passes NonEmptyString "  ",

  rejects_empty = fails NonEmptyString "",
  rejects_number = fails NonEmptyString 42,
  rejects_bool = fails NonEmptyString true,
  rejects_null = fails NonEmptyString null,
}
in

# ── RelativePath ──────────────────────────────────────────────────
let relative_path_tests = {
  valid_simple = passes RelativePath "src",
  valid_nested = passes RelativePath "src/main.rs",
  valid_dotted = passes RelativePath "./src",
  valid_parent = passes RelativePath "../lib",

  rejects_absolute = fails RelativePath "/nix/store/something",
  rejects_empty = fails RelativePath "",
  rejects_number = fails RelativePath 42,
  rejects_null = fails RelativePath null,
}
in

# ── ModuleRef ─────────────────────────────────────────────────────
let module_ref_tests = {
  valid_name = passes ModuleRef "desktop",
  valid_path = passes ModuleRef "./modules/desktop.nix",

  rejects_empty = fails ModuleRef "",
  rejects_number = fails ModuleRef 42,
  rejects_null = fails ModuleRef null,
}
in

# ── Collect all results ──────────────────────────────────────────
let all_tests =
  std.record.to_array system_tests
  @ std.record.to_array name_tests
  @ std.record.to_array non_empty_string_tests
  @ std.record.to_array relative_path_tests
  @ std.record.to_array module_ref_tests
in

let failures =
  all_tests
  |> std.array.filter (fun entry => entry.value == false)
  |> std.array.map (fun entry => entry.field)
in

if std.array.length failures > 0 then
  std.fail_with "FAILED tests: %{std.string.join ", " failures}"
else
  "All %{std.to_string (std.array.length all_tests)} common contract tests passed."
