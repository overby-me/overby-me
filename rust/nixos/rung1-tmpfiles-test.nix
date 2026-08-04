# NixOS VM test for rung 1 (docs/ROADMAP.md "Ship one increment"): run ONE rust
# component under the C systemd manager.
#
# Boots a NixOS VM whose PID 1 is the stock C systemd, but whose
# systemd-tmpfiles binary is the rust build, and verifies that the boot-time
# systemd-tmpfiles-setup applies a tmpfiles.d rule using the rust binary while
# PID 1 stays the C manager. This is the in-VM precursor to running a rust
# component under C PID 1 on a real machine (which opens the production feedback
# channel VM tests cannot).
#
# Run with: nix build .#checks.x86_64-linux.rust-rung1-tmpfiles -L
{pkgs}: let
  # Stock C systemd with ONLY systemd-tmpfiles replaced by the rust build.
  # Everything else (PID 1, every other helper, the module-interface passthru)
  # is the C package, so NixOS still drives it as the stock systemd and PID 1
  # remains the C manager. cp -L dereferences the cargo build's per-crate
  # symlinks so the result references only glibc.
  hybridSystemd =
    pkgs.runCommand "systemd-rung1-tmpfiles-${pkgs.systemd.version}" {
      passthru = pkgs.systemd.passthru;
      # Single-output copy: keep systemd's meta but pin outputsToInstall to
      # "out" (the C package is multi-output with a separate man/dev; the test
      # does not need those and referencing a missing output would fail).
      meta = (pkgs.systemd.meta or {}) // {outputsToInstall = ["out"];};
    } ''
      cp -a ${pkgs.systemd} $out
      chmod -R u+w $out
      for p in bin/systemd-tmpfiles lib/systemd/systemd-tmpfiles; do
        if [ -e "$out/$p" ]; then
          rm -f "$out/$p"
          cp -L ${pkgs.rust-systemd}/bin/systemd-tmpfiles "$out/$p"
          chmod u+w "$out/$p"
        fi
      done
    '';
in
  pkgs.testers.nixosTest {
    name = "rust-rung1-tmpfiles";

    nodes.machine = {...}: {
      system.stateVersion = "25.11";

      # PID 1 is the C manager; only systemd-tmpfiles is the rust build.
      systemd.package = hybridSystemd;

      # A tmpfiles.d rule the boot-time systemd-tmpfiles-setup must apply with
      # the rust binary.
      systemd.tmpfiles.rules = [
        "d /run/rung1-tmpfiles 0755 root root -"
        "f /run/rung1-tmpfiles/created 0644 root root -"
      ];
    };

    testScript = ''
      machine.wait_for_unit("multi-user.target")

      # The rust systemd-tmpfiles applied the boot-time rule under the C manager.
      machine.succeed("test -d /run/rung1-tmpfiles")
      machine.succeed("test -f /run/rung1-tmpfiles/created")

      # The swapped binary is the rust build (version 0.1.0), not C (260),
      # proving the substitution took effect rather than silently falling back.
      machine.succeed("systemd-tmpfiles --version | grep -q '0\\.1\\.0'")

      # ...while PID 1 remains the C systemd manager.
      version = machine.succeed("systemctl --version")
      assert "260" in version, f"expected C systemd 260 as PID 1, got: {version!r}"
    '';
  }
