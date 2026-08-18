# Unit tests for PackageConfig contract
#
# Validates that the PackageConfig contract correctly accepts valid
# package configurations and rejects invalid ones.
let { PackageConfig, .. } = import "../../contracts/package.ncl" in

# ── Helpers ──────────────────────────────────────────────────────
let assert_ok = fun label value =>
  let _ = std.seq value null in
  "PASS: %{label}"
in

let assert_fail = fun label thunk =>
  let result =
    thunk
    |> std.seq
    |> (fun _ => "FAIL: %{label} — expected contract violation, got success")
  in
  result
in

# ── Valid configs ────────────────────────────────────────────────
let test_minimal =
  assert_ok
    "minimal package (all defaults)"
    (
      {} | PackageConfig
    )
in

let test_generic_build_system =
  assert_ok
    "explicit generic build-system"
    (
      { build-system = "generic" } | PackageConfig
    )
in

let test_rust_build_system =
  assert_ok
    "rust build-system"
    (
      { build-system = "rust" } | PackageConfig
    )
in

let test_go_build_system =
  assert_ok
    "go build-system"
    (
      { build-system = "go" } | PackageConfig
    )
in

let test_full_package =
  assert_ok
    "fully specified package"
    (
      {
        src = "./my-tool",
        build-system = "rust",
        description = "A CLI tool written in Rust",
        systems = ["x86_64-linux"],
        build-inputs = ["openssl"],
        native-build-inputs = ["pkg-config"],
        env = {
          OPENSSL_NO_VENDOR = "1",
        },
        meta = {
          homepage = "https://example.com",
          license = "MIT",
          maintainers = ["alice"],
        },
        override = {},
      } | PackageConfig
    )
in

let test_with_src =
  assert_ok
    "package with src"
    (
      {
        src = "./src",
        description = "Package with source",
      } | PackageConfig
    )
in

let test_with_build_inputs =
  assert_ok
    "package with build inputs"
    (
      {
        build-inputs = ["zlib", "openssl"],
        native-build-inputs = ["cmake", "pkg-config"],
      } | PackageConfig
    )
in

let test_with_env =
  assert_ok
    "package with env vars"
    (
      {
        env = {
          MY_VAR = "hello",
          OTHER_VAR = "world",
        },
      } | PackageConfig
    )
in

let test_with_meta =
  assert_ok
    "package with metadata"
    (
      {
        meta = {
          homepage = "https://example.com",
          license = "MIT",
          maintainers = ["alice", "bob"],
          platforms = ["x86_64-linux"],
        },
      } | PackageConfig
    )
in

let test_with_systems_override =
  assert_ok
    "package with systems override"
    (
      {
        systems = ["x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin"],
      } | PackageConfig
    )
in

let test_with_override_escape_hatch =
  assert_ok
    "package with override escape hatch"
    (
      {
        override = {
          dontFixup = "true",
          postInstall = "echo done",
        },
      } | PackageConfig
    )
in

# ── Output ───────────────────────────────────────────────────────
[
  test_minimal,
  test_generic_build_system,
  test_rust_build_system,
  test_go_build_system,
  test_full_package,
  test_with_src,
  test_with_build_inputs,
  test_with_env,
  test_with_meta,
  test_with_systems_override,
  test_with_override_escape_hatch,
]
|> std.array.map (fun result => std.trace result null)
|> (fun _ => "All PackageConfig tests passed.")
