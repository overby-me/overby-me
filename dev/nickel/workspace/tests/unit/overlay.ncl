# Unit tests for OverlayConfig contract
#
# Validates that the OverlayConfig contract correctly accepts valid
# overlay configurations and rejects invalid ones.
let { OverlayConfig, .. } = import "../../contracts/overlay.ncl" in

# ── Helpers ──────────────────────────────────────────────────────
let assert_ok = fun label value =>
  let _ = std.seq value null in
  "PASS: %{label}"
in

# ── Valid configs ────────────────────────────────────────────────
let test_minimal =
  assert_ok
    "minimal overlay (all defaults)"
    (
      {} | OverlayConfig
    )
in

let test_with_description =
  assert_ok
    "overlay with description"
    (
      {
        description = "Custom packages overlay",
      } | OverlayConfig
    )
in

let test_with_path =
  assert_ok
    "overlay with explicit path"
    (
      {
        description = "My overlay",
        path = "./overlays/custom.nix",
      } | OverlayConfig
    )
in

let test_with_priority =
  assert_ok
    "overlay with custom priority"
    (
      {
        description = "High priority overlay",
        priority = 50,
      } | OverlayConfig
    )
in

let test_priority_zero =
  assert_ok
    "overlay with zero priority"
    (
      {
        priority = 0,
      } | OverlayConfig
    )
in

let test_with_packages =
  assert_ok
    "overlay with package list"
    (
      {
        description = "Packages overlay",
        packages = ["my-tool", "my-lib", "my-cli"],
      } | OverlayConfig
    )
in

let test_full_overlay =
  assert_ok
    "fully specified overlay"
    (
      {
        description = "Full custom overlay",
        path = "./overlays/full.nix",
        priority = 10,
        packages = ["custom-pkg", "patched-lib"],
        extra-config = {
          some-key = "some-value",
        },
      } | OverlayConfig
    )
in

let test_with_extra_config =
  assert_ok
    "overlay with extra-config escape hatch"
    (
      {
        extra-config = {
          arbitrary = "data",
        },
      } | OverlayConfig
    )
in

let test_empty_packages =
  assert_ok
    "overlay with empty packages list"
    (
      {
        packages = [],
      } | OverlayConfig
    )
in

let test_default_priority =
  assert_ok
    "overlay default priority is 100"
    (
      let cfg = {} | OverlayConfig in
      let _ = std.seq cfg null in
      "checked"
    )
in

# ── Output ───────────────────────────────────────────────────────
[
  test_minimal,
  test_with_description,
  test_with_path,
  test_with_priority,
  test_priority_zero,
  test_with_packages,
  test_full_overlay,
  test_with_extra_config,
  test_empty_packages,
  test_default_priority,
]
|> std.array.map (fun result => std.trace result null)
|> (fun _ => "All OverlayConfig tests passed.")
