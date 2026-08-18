# NixOS VM test for a second rung-1 DAEMON under the C systemd manager:
# systemd-resolved (DNS), via the reusable rung1.nix module.
#
# PID 1 and the systemd package stay stock C; the module's
# services.rustSystemdRung1.resolved toggle redirects only
# systemd-resolved.service to the rust systemd-resolved binary. resolved is
# Type=notify-reload with a D-Bus name (org.freedesktop.resolve1) and two sockets
# (varlink/monitor), but is NOT boot-critical. The test verifies the rust daemon
# reaches active under C PID 1; the testScript prints the service journal so any
# failure is self-diagnosing. This validates the deployable module toggle.
#
# Run with: nix build .#checks.x86_64-linux.oxidized-nixos-rung1-resolved -L
{pkgs}:
pkgs.testers.nixosTest {
  name = "oxidized-nixos-rung1-resolved";

  nodes.machine = {lib, ...}: {
    imports = [./rung1.nix];
    system.stateVersion = "25.11";

    # Ensure the resolved service exists, then run the rust binary via the
    # module toggle.
    services.resolved.enable = lib.mkForce true;
    services.rustSystemdRung1.resolved.enable = true;
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target")

    # Diagnostic: surface the rust daemon's exact behaviour.
    print(machine.execute("systemctl --no-pager -l status systemd-resolved.service || true")[1])
    print(machine.execute("journalctl --no-pager -b -u systemd-resolved.service || true")[1])

    # PID 1 is the C systemd manager.
    version = machine.succeed("systemctl --version")
    assert "260" in version, f"expected C systemd 260 as PID 1, got: {version!r}"

    # The redirected service runs the rust binary...
    machine.succeed("systemctl cat systemd-resolved.service | grep -q oxidized-systemd")

    # ...and the rust Type=notify-reload daemon reached active (signaled READY).
    machine.wait_for_unit("systemd-resolved.service")
    machine.succeed("systemctl is-active systemd-resolved.service")
  '';
}
