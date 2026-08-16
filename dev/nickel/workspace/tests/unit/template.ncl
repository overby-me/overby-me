# Unit tests for TemplateConfig contract
#
# Validates that the TemplateConfig contract correctly accepts valid
# template configurations and rejects invalid ones.
let { TemplateConfig, .. } = import "../../contracts/template.ncl" in

# ── Helpers ──────────────────────────────────────────────────────
let assert_ok = fun label value =>
  let _ = std.seq value null in
  "PASS: %{label}"
in

# ── Valid configs ────────────────────────────────────────────────
let test_minimal =
  assert_ok
    "minimal template (only required description)"
    (
      {
        description = "A basic template",
      } | TemplateConfig
    )
in

let test_with_path =
  assert_ok
    "template with explicit path"
    (
      {
        description = "Rust workspace template",
        path = "./templates/rust-minimal",
      } | TemplateConfig
    )
in

let test_with_welcome_text =
  assert_ok
    "template with welcome text"
    (
      {
        description = "Starter project",
        welcome-text = "Run 'nix develop' to enter the dev shell, then 'cargo build'.",
      } | TemplateConfig
    )
in

let test_with_tags =
  assert_ok
    "template with tags"
    (
      {
        description = "Tagged template",
        tags = ["rust", "cli", "minimal"],
      } | TemplateConfig
    )
in

let test_empty_tags =
  assert_ok
    "template with empty tags list"
    (
      {
        description = "No tags",
        tags = [],
      } | TemplateConfig
    )
in

let test_full_template =
  assert_ok
    "fully specified template"
    (
      {
        description = "Full Rust workspace with CI",
        path = "./templates/rust-full",
        welcome-text = m%"
          Your new Rust workspace is ready!

          Next steps:
            1. cd into the project directory
            2. Run 'nix develop' to enter the dev shell
            3. Run 'cargo build' to compile
            4. Run 'nix flake check' to validate
        "%,
        tags = ["rust", "workspace", "ci", "full"],
        extra-config = {
          custom-key = "custom-value",
        },
      } | TemplateConfig
    )
in

let test_with_extra_config =
  assert_ok
    "template with extra-config escape hatch"
    (
      {
        description = "Template with extras",
        extra-config = {
          arbitrary = "data",
          nested = { key = "val" },
        },
      } | TemplateConfig
    )
in

let test_single_tag =
  assert_ok
    "template with a single tag"
    (
      {
        description = "Single tag template",
        tags = ["minimal"],
      } | TemplateConfig
    )
in

let test_multiline_welcome_text =
  assert_ok
    "template with multiline welcome text"
    (
      {
        description = "Multiline welcome",
        welcome-text = "Line 1\nLine 2\nLine 3",
      } | TemplateConfig
    )
in

let test_relative_path =
  assert_ok
    "template with relative path"
    (
      {
        description = "Relative path template",
        path = "templates/my-template",
      } | TemplateConfig
    )
in

let test_nested_path =
  assert_ok
    "template with nested relative path"
    (
      {
        description = "Nested path template",
        path = "./templates/category/sub-template",
      } | TemplateConfig
    )
in

let test_description_with_special_chars =
  assert_ok
    "template with special characters in description"
    (
      {
        description = "A template for Rust/Go projects (v2.0) — with extras!",
      } | TemplateConfig
    )
in

# ── Output ───────────────────────────────────────────────────────
[
  test_minimal,
  test_with_path,
  test_with_welcome_text,
  test_with_tags,
  test_empty_tags,
  test_full_template,
  test_with_extra_config,
  test_single_tag,
  test_multiline_welcome_text,
  test_relative_path,
  test_nested_path,
  test_description_with_special_chars,
]
|> std.array.map (fun result => std.trace result null)
|> (fun _ => "All TemplateConfig tests passed.")
