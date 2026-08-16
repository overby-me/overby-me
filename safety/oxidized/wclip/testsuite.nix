# Command-line behaviour tests for rust-wclip.
#
# These run in the Nix sandbox where there is no Wayland compositor, so they
# cover argument parsing, help/version output, and graceful failure modes. The
# wire-protocol copy/paste roundtrip is covered by the crate's own tests, which
# run during the package build.
#
# Run with: nix build .#checks.x86_64-linux.rust-wclip-test-{name}
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-wclip-test-${name}" {
  nativeBuildInputs = [pkgs.rust-wclip-dev pkgs.coreutils];
} ''
  wclip=${pkgs.rust-wclip-dev}/bin/wclip
  set -u

  # Helper: assert a command exits with an expected status.
  expect_status() {
    local want="$1"; shift
    set +e
    "$@" >"$TMPDIR/out" 2>"$TMPDIR/err"
    local got=$?
    set -e
    if [ "$got" != "$want" ]; then
      echo "FAIL: expected exit $want, got $got for: $*"
      echo "stdout:"; cat "$TMPDIR/out"
      echo "stderr:"; cat "$TMPDIR/err"
      exit 1
    fi
  }

  export TMPDIR="$(mktemp -d)"
  unset WAYLAND_SOCKET || true

  case "${name}" in
    version)
      expect_status 0 "$wclip" -version
      grep -q '^wclip ' "$TMPDIR/out"
      ;;
    version-format)
      "$wclip" -version >"$TMPDIR/out"
      grep -Eq '^wclip [0-9]+\.[0-9]+\.[0-9]+$' "$TMPDIR/out"
      ;;
    help)
      expect_status 0 "$wclip" -help
      grep -q 'Usage:' "$TMPDIR/out"
      ;;
    help-mentions-clipboard)
      "$wclip" -help >"$TMPDIR/out"
      grep -q 'selection clipboard' "$TMPDIR/out"
      ;;
    help-exit-zero)
      expect_status 0 "$wclip" -h
      ;;
    invalid-option)
      expect_status 2 "$wclip" -bogus
      grep -qi 'invalid option' "$TMPDIR/err"
      ;;
    secondary-selection)
      echo x | { expect_status 2 "$wclip" -selection secondary; }
      grep -qi 'secondary' "$TMPDIR/err"
      ;;
    bad-loops)
      echo x | { expect_status 2 "$wclip" -loops abc; }
      grep -qi 'loop count' "$TMPDIR/err"
      ;;
    missing-argument)
      expect_status 2 "$wclip" -selection
      grep -qi 'requires an argument' "$TMPDIR/err"
      ;;
    no-display)
      export XDG_RUNTIME_DIR="$TMPDIR"
      export WAYLAND_DISPLAY="wclip-nonexistent-display"
      expect_status 1 "$wclip" -o
      grep -qi 'connect to Wayland' "$TMPDIR/err"
      ;;
    selection-abbreviation)
      # 'c' must resolve to clipboard and proceed to (failing) connection,
      # not be rejected as an unknown selection.
      export XDG_RUNTIME_DIR="$TMPDIR"
      export WAYLAND_DISPLAY="wclip-nonexistent-display"
      expect_status 1 "$wclip" -o -sel c
      grep -qi 'connect to Wayland' "$TMPDIR/err"
      ;;
    *)
      echo "unknown test: ${name}"
      exit 1
      ;;
  esac

  touch $out
''
