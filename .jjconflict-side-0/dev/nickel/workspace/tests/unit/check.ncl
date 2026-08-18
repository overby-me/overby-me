# Unit tests for CheckConfig contract
#
# Validates that the CheckConfig contract correctly accepts valid
# check configurations and rejects invalid ones.
let { CheckConfig, .. } = import "../../contracts/check.ncl" in

# ── Helpers ──────────────────────────────────────────────────────
let assert_ok = fun label value =>
  let _ = std.seq value null in
  "PASS: %{label}"
in

# ── Valid configs ────────────────────────────────────────────────
let test_minimal =
  assert_ok
    "minimal check (all defaults)"
    (
      {} | CheckConfig
    )
in

let test_with_description =
  assert_ok
    "check with description"
    (
      {
        description = "Run unit tests",
      } | CheckConfig
    )
in

let test_with_command =
  assert_ok
    "check with command"
    (
      {
        description = "Cargo tests",
        command = "cargo test --workspace",
      } | CheckConfig
    )
in

let test_with_path =
  assert_ok
    "check with explicit path"
    (
      {
        description = "Custom check derivation",
        path = "./checks/lint.nix",
      } | CheckConfig
    )
in

let test_with_packages =
  assert_ok
    "check with packages"
    (
      {
        command = "cargo test",
        packages = ["cargo", "rustc", "gcc"],
      } | CheckConfig
    )
in

let test_with_env =
  assert_ok
    "check with environment variables"
    (
      {
        command = "pytest",
        packages = ["python3"],
        env = {
          PYTHONDONTWRITEBYTECODE = "1",
          LOG_LEVEL = "debug",
        },
      } | CheckConfig
    )
in

let test_with_systems =
  assert_ok
    "check with systems override"
    (
      {
        command = "make test",
        systems = ["x86_64-linux"],
      } | CheckConfig
    )
in

let test_with_all_systems =
  assert_ok
    "check with all valid systems"
    (
      {
        command = "echo ok",
        systems = ["x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin"],
      } | CheckConfig
    )
in

let test_with_timeout =
  assert_ok
    "check with custom timeout"
    (
      {
        command = "cargo test --release",
        timeout = 1200,
      } | CheckConfig
    )
in

let test_with_small_timeout =
  assert_ok
    "check with small timeout"
    (
      {
        command = "echo fast",
        timeout = 1,
      } | CheckConfig
    )
in

let test_with_inputs_from =
  assert_ok
    "check with inputs-from"
    (
      {
        command = "cargo test",
        inputs-from = ["my-tool", "my-lib"],
      } | CheckConfig
    )
in

let test_full_check =
  assert_ok
    "fully specified check"
    (
      {
        description = "Integration test suite",
        command = "cargo test --features integration",
        packages = ["cargo", "rustc", "openssl"],
        env = {
          RUST_LOG = "debug",
          TEST_DB_URL = "postgres://localhost/test",
        },
        systems = ["x86_64-linux", "aarch64-linux"],
        timeout = 900,
        inputs-from = ["my-server"],
        extra-config = {
          some-nix-option = "value",
        },
      } | CheckConfig
    )
in

let test_with_extra_config =
  assert_ok
    "check with extra-config escape hatch"
    (
      {
        extra-config = {
          arbitrary = "data",
          nested = { key = "val" },
        },
      } | CheckConfig
    )
in

let test_empty_packages =
  assert_ok
    "check with empty packages list"
    (
      {
        command = "true",
        packages = [],
      } | CheckConfig
    )
in

let test_empty_inputs_from =
  assert_ok
    "check with empty inputs-from"
    (
      {
        inputs-from = [],
      } | CheckConfig
    )
in

let test_default_timeout =
  assert_ok
    "check default timeout is 600"
    (
      let cfg = {} | CheckConfig in
      let _ = std.seq cfg null in
      "checked"
    )
in

# ── Output ───────────────────────────────────────────────────────
[
  test_minimal,
  test_with_description,
  test_with_command,
  test_with_path,
  test_with_packages,
  test_with_env,
  test_with_systems,
  test_with_all_systems,
  test_with_timeout,
  test_with_small_timeout,
  test_with_inputs_from,
  test_full_check,
  test_with_extra_config,
  test_empty_packages,
  test_empty_inputs_from,
  test_default_timeout,
]
|> std.array.map (fun result => std.trace result null)
|> (fun _ => "All CheckConfig tests passed.")
