# NixOS VM test for rung 1 (docs/ROADMAP.md "Ship one increment"): run ONE rust
# component under the C systemd manager.
#
# PID 1 and the whole systemd package stay stock C. Only the boot-time
# systemd-tmpfiles-setup service is redirected to run the rust systemd-tmpfiles
# binary (the incremental-adoption shape, and the real-machine mechanism). The
# test verifies the rust binary applies the full boot tmpfiles.d set -- a real
# system's rules, including POSIX ACLs, plus a custom marker rule -- and that
# the service succeeds, while PID 1 remains the C manager.
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

      # A tmpfiles.d rule the redirected service must apply with the rust binary.
      systemd.tmpfiles.rules = [
        "d /run/rung1-tmpfiles 0755 root root -"
        "f /run/rung1-tmpfiles/created 0644 root root -"
      ];
    };

    testScript = ''
      machine.wait_for_unit("multi-user.target")

      # The redirected boot service ran the rust binary and succeeded on the
      # full NixOS tmpfiles.d set (including POSIX ACL rules).
      machine.wait_for_unit("systemd-tmpfiles-setup.service")
      machine.succeed("systemctl cat systemd-tmpfiles-setup.service | grep -q rust-systemd")

      # The rust systemd-tmpfiles applied the custom rule.
      machine.succeed("test -d /run/rung1-tmpfiles")
      machine.succeed("test -f /run/rung1-tmpfiles/created")

      # ...while PID 1 remains the C systemd manager.
      version = machine.succeed("systemctl --version")
      assert "260" in version, f"expected C systemd 260 as PID 1, got: {version!r}"
    '';
  }
