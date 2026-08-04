# NixOS VM test for rung 1 (docs/ROADMAP.md "Ship one increment"): run ONE rust
# component under the C systemd manager.
#
# PID 1 and the whole systemd package stay stock C. Only the boot-time
# systemd-tmpfiles-setup service is redirected to run the rust systemd-tmpfiles
# binary (the incremental-adoption shape, and the real-machine mechanism).
#
# STATUS: currently RED. The VM boots cleanly under C PID 1, but the rust
# systemd-tmpfiles fails when applying the COMPLETE NixOS tmpfiles.d set at boot
# (a simple one-rule config works; the full system set does not). The testScript
# prints the failed service's journal so the exact root-context error can be
# pinned. This is a real rust-tmpfiles boot-compatibility gap tracked in
# docs/TEST-OVERRIDES.md; the check is kept so the gap stays visible and the fix
# can be validated. Do NOT mark green until the rust binary applies the full set.
#
# Run with: nix build .#checks.x86_64-linux.rust-rung1-tmpfiles -L
{pkgs}: let
  rustTmpfiles = "${pkgs.rust-systemd}/bin/systemd-tmpfiles";
in
  pkgs.testers.nixosTest {
    name = "rust-rung1-tmpfiles";

    nodes.machine = {lib, ...}: {
      system.stateVersion = "25.11";

      # The C manager runs the rust systemd-tmpfiles for the boot-time setup.
      systemd.services.systemd-tmpfiles-setup.serviceConfig.ExecStart = lib.mkForce [
        ""
        "${rustTmpfiles} --create --remove --boot --exclude-prefix=/dev"
      ];

      systemd.tmpfiles.rules = [
        "d /run/rung1-tmpfiles 0755 root root -"
        "f /run/rung1-tmpfiles/created 0644 root root -"
      ];
    };

    testScript = ''
      machine.wait_for_unit("multi-user.target")

      # Diagnostic: surface the rust binary's exact behaviour on the full set.
      print(machine.execute("systemctl --no-pager -l status systemd-tmpfiles-setup.service || true")[1])
      print(machine.execute("journalctl --no-pager -b -u systemd-tmpfiles-setup.service || true")[1])

      # PID 1 is the C systemd manager (this holds regardless of the gap).
      version = machine.succeed("systemctl --version")
      assert "260" in version, f"expected C systemd 260 as PID 1, got: {version!r}"

      # Rung-1 goal (RED until rust tmpfiles applies the full boot set): the
      # redirected service ran the rust binary and applied the rule.
      machine.succeed("systemctl cat systemd-tmpfiles-setup.service | grep -q rust-systemd")
      machine.succeed("systemctl is-active systemd-tmpfiles-setup.service")
      machine.succeed("test -f /run/rung1-tmpfiles/created")
    '';
  }
