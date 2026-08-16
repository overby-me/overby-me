# NixOS VM test for rung 1b (docs/ROADMAP.md "Ship one increment"): a SECOND
# rust component under the C systemd manager, after rung1-tmpfiles.
#
# PID 1 and the systemd package stay stock C, and so does the boot
# systemd-sysusers.service (which creates the system's own users -- redirecting
# THAT to rust sysusers wedges boot, since this port creates users via
# useradd/groupadd rather than writing /etc/passwd directly like C, and the
# system's users must exist for early boot). Instead a dedicated oneshot runs the
# rust systemd-sysusers under the C manager on a controlled config, proving the
# component runs and creates accounts under C PID 1.
#
# rust systemd-sysusers shells out to useradd/groupadd/chage, resolved from the
# baked SHADOW_BIN path so they work from a minimal service $PATH (mirrors the
# setfacl fix for tmpfiles). The testScript prints the service journal so any
# failure is self-diagnosing.
#
# Run with: nix build .#checks.x86_64-linux.rust-rung1-sysusers -L
{pkgs}: let
  rustSysusers = "${pkgs.rust-systemd}/bin/systemd-sysusers";
in
  pkgs.testers.nixosTest {
    name = "rust-rung1-sysusers";

    nodes.machine = {...}: {
      system.stateVersion = "25.11";

      # A controlled sysusers.d rule the rust binary must apply. Fixed ids avoid
      # allocation divergence between the ports.
      environment.etc."sysusers.d/rung1.conf".text = ''
        g rung1grp 60123
        u rung1usr 60123:60123 "Rung 1 test user" /var/empty /sbin/nologin
      '';

      # Dedicated oneshot: run the rust systemd-sysusers under the C manager,
      # WITHOUT replacing the boot systemd-sysusers.service.
      systemd.services.rung1-rust-sysusers = {
        description = "Rung 1b: rust systemd-sysusers under C PID 1";
        wantedBy = ["multi-user.target"];
        after = ["systemd-sysusers.service"];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = "${rustSysusers} /etc/sysusers.d/rung1.conf";
        };
      };
    };

    testScript = ''
      machine.wait_for_unit("multi-user.target")

      # Diagnostic: surface the rust binary's exact behaviour.
      print(machine.execute("systemctl --no-pager -l status rung1-rust-sysusers.service || true")[1])
      print(machine.execute("journalctl --no-pager -b -u rung1-rust-sysusers.service || true")[1])

      # PID 1 is the C systemd manager.
      version = machine.succeed("systemctl --version")
      assert "260" in version, f"expected C systemd 260 as PID 1, got: {version!r}"

      # The rust systemd-sysusers ran and created the system group + user.
      machine.succeed("systemctl is-active rung1-rust-sysusers.service")
      machine.succeed("getent group rung1grp")
      machine.succeed("getent passwd rung1usr")
    '';
  }
