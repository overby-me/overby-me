# NixOS VM test for the first rung-1 DAEMON under the C systemd manager:
# systemd-timesyncd, via the reusable rung1.nix module.
#
# PID 1 and the systemd package stay stock C; the module's
# services.rustSystemdRung1.timesyncd toggle redirects only
# systemd-timesyncd.service to the rust systemd-timesyncd binary. timesyncd is a
# long-running Type=notify daemon (it must sd_notify READY=1 or the service hangs
# 'activating') but is NOT boot-critical. The test verifies the rust daemon
# reaches active under C PID 1; the testScript prints the service journal so any
# failure is self-diagnosing. This validates the deployable module toggle.
#
# Run with: nix build .#checks.x86_64-linux.rust-rung1-timesyncd -L
{pkgs}:
pkgs.testers.nixosTest {
  name = "rust-rung1-timesyncd";

  nodes.machine = {lib, ...}: {
    imports = [./rung1.nix];
    system.stateVersion = "25.11";

    # Ensure the timesyncd service exists, then run the rust binary via the
    # module toggle.
    services.timesyncd.enable = lib.mkForce true;
    services.rustSystemdRung1.timesyncd.enable = true;
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target")

    # Diagnostic: surface the rust daemon's exact behaviour.
    print(machine.execute("systemctl --no-pager -l status systemd-timesyncd.service || true")[1])
    print(machine.execute("journalctl --no-pager -b -u systemd-timesyncd.service || true")[1])

    # PID 1 is the C systemd manager.
    version = machine.succeed("systemctl --version")
    assert "260" in version, f"expected C systemd 260 as PID 1, got: {version!r}"

    # The redirected service runs the rust binary...
    machine.succeed("systemctl cat systemd-timesyncd.service | grep -q rust-systemd")

    # ...and the rust Type=notify daemon reached active (it signaled READY=1),
    # proving a long-running rust daemon works under the C manager.
    machine.wait_for_unit("systemd-timesyncd.service")
    machine.succeed("systemctl is-active systemd-timesyncd.service")
  '';
}
