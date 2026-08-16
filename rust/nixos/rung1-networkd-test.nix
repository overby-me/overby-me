# NixOS VM test for the most meaningful rung-1 daemon under the C systemd
# manager: systemd-networkd (network configuration), via the reusable rung1.nix
# module.
#
# PID 1 and the systemd package stay stock C; the module's
# services.rustSystemdRung1.networkd toggle redirects only
# systemd-networkd.service to the rust systemd-networkd binary. networkd is
# Type=notify-reload with a D-Bus name (org.freedesktop.network1) and four
# sockets, and configures the network over netlink. It is not boot-critical
# (wait-online is disabled here). The test verifies the rust daemon reaches
# active under C PID 1; the testScript prints the service journal so any failure
# is self-diagnosing. This validates the deployable module toggle.
#
# Run with: nix build .#checks.x86_64-linux.rust-rung1-networkd -L
{pkgs}:
pkgs.testers.nixosTest {
  name = "rust-rung1-networkd";

  nodes.machine = {lib, ...}: {
    imports = [./rung1.nix];
    system.stateVersion = "25.11";

    # Use networkd as the network backend, then run the rust binary via the
    # module toggle.
    networking.useNetworkd = true;
    networking.useDHCP = false;
    services.rustSystemdRung1.networkd.enable = true;
    # Don't let network-online block boot if the link isn't configured.
    systemd.services.systemd-networkd-wait-online.enable = lib.mkForce false;
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target")

    # Diagnostic: surface the rust daemon's exact behaviour.
    print(machine.execute("systemctl --no-pager -l status systemd-networkd.service || true")[1])
    print(machine.execute("journalctl --no-pager -b -u systemd-networkd.service || true")[1])

    # PID 1 is the C systemd manager.
    version = machine.succeed("systemctl --version")
    assert "260" in version, f"expected C systemd 260 as PID 1, got: {version!r}"

    # The redirected service runs the rust binary...
    machine.succeed("systemctl cat systemd-networkd.service | grep -q rust-systemd")

    # ...and the rust Type=notify-reload daemon reached active (signaled READY).
    machine.wait_for_unit("systemd-networkd.service")
    machine.succeed("systemctl is-active systemd-networkd.service")
  '';
}
