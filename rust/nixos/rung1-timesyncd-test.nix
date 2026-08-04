# NixOS VM test for the first rung-1 DAEMON under the C systemd manager:
# systemd-timesyncd (after the tmpfiles/sysusers oneshots).
#
# PID 1 and the systemd package stay stock C; only systemd-timesyncd.service is
# redirected to the rust systemd-timesyncd binary. timesyncd is a long-running
# Type=notify daemon (it must sd_notify READY=1 or the service hangs
# 'activating'), but it is NOT boot-critical, so a failure fails the service in
# isolation rather than wedging boot. The test verifies the rust daemon reaches
# active (signaled READY) under C PID 1; the testScript prints the service
# journal so any failure is self-diagnosing.
#
# Run with: nix build .#checks.x86_64-linux.rust-rung1-timesyncd -L
{pkgs}: let
  rustTimesyncd = "${pkgs.rust-systemd}/bin/systemd-timesyncd";
in
  pkgs.testers.nixosTest {
    name = "rust-rung1-timesyncd";

    nodes.machine = {lib, ...}: {
      system.stateVersion = "25.11";

      # Ensure the timesyncd service exists, then run the rust binary for it.
      services.timesyncd.enable = lib.mkForce true;
      systemd.services.systemd-timesyncd.serviceConfig.ExecStart = lib.mkForce [
        ""
        rustTimesyncd
      ];
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
