# NixOS VM test for rung 1 (docs/ROADMAP.md "Ship one increment"): run ONE rust
# component under the C systemd manager, via the reusable rung1.nix module.
#
# PID 1 and the whole systemd package stay stock C; the module's
# services.rustSystemdRung1.tmpfiles toggle redirects only the boot
# systemd-tmpfiles-setup service to the rust systemd-tmpfiles. The test verifies
# it applies the full boot tmpfiles.d set (POSIX ACLs included) plus a custom
# marker rule and succeeds, while PID 1 remains the C manager. This validates the
# deployable module, not just an inline override.
#
# Run with: nix build .#checks.x86_64-linux.rust-rung1-tmpfiles -L
{pkgs}:
pkgs.testers.nixosTest {
  name = "rust-rung1-tmpfiles";

  nodes.machine = {...}: {
    imports = [./rung1.nix];
    system.stateVersion = "25.11";

    # The C manager runs the rust systemd-tmpfiles for the boot-time setup.
    services.rustSystemdRung1.tmpfiles.enable = true;

    # A tmpfiles.d rule the redirected service must apply with the rust binary.
    systemd.tmpfiles.rules = [
      "d /run/rung1-tmpfiles 0755 root root -"
      "f /run/rung1-tmpfiles/created 0644 root root -"
    ];
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target")

    # The redirected boot service ran the rust binary and succeeded on the full
    # NixOS tmpfiles.d set (including POSIX ACL rules).
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
