# Run a single flatpak integration test inside a NixOS VM.
#
# Boots a NixOS VM with bubblewrap, D-Bus, and rust-flatpak installed,
# copies the test scripts in, and runs the specified test as a regular user.
#
# Run with: nix build .#checks.x86_64-linux.rust-flatpak-vm-{name}
# Example:  nix build .#checks.x86_64-linux.rust-flatpak-vm-run-hello
{
  pkgs,
  name,
  testTimeout ? 600,
}:
pkgs.testers.nixosTest {
  name = "rust-flatpak-vm-${name}";

  nodes.machine = {pkgs, ...}: {
    environment = {
      systemPackages = [
        pkgs.rust-flatpak-dev
        pkgs.bubblewrap
        pkgs.xdg-dbus-proxy
        pkgs.coreutils
        pkgs.gnugrep
        pkgs.gnused
        pkgs.gawk
        pkgs.diffutils
        pkgs.bash
        pkgs.findutils
        pkgs.procps
        pkgs.gnutar
        pkgs.gzip
        pkgs.util-linux # for unshare, mount, etc.
        pkgs.python3 # for HTTP test server (Cat 13 network tests)
      ];

      # Copy test scripts into /etc/flatpak-vmtests so they're available in the VM
      etc."flatpak-vmtests/libtest-nix.sh" = {
        source = ./vmtests/libtest-nix.sh;
        mode = "0755";
      };
      etc."flatpak-vmtests/vm-${name}.sh" = {
        source = ./vmtests/vm-${name}.sh;
        mode = "0755";
      };
    };

    # Ensure /etc/timezone exists (bwrap tries to bind-mount it)
    time.timeZone = "UTC";

    # D-Bus session bus (needed for portal/document/permission tests)
    services.dbus.enable = true;

    # bubblewrap needs unprivileged user namespaces
    security.unprivilegedUsernsClone = true;

    # Test user (sandbox needs non-root)
    users.users.testuser = {
      isNormalUser = true;
      extraGroups = ["wheel"];
      password = "test";
    };

    virtualisation = {
      memorySize = 2048;
      cores = 2;
    };
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target", timeout=120)

    # Prepare working directory and copy test scripts
    machine.succeed(
        "mkdir -p /tmp/flatpak-vmtest/vmtests && "
        "cp /etc/flatpak-vmtests/libtest-nix.sh /tmp/flatpak-vmtest/vmtests/ && "
        "cp /etc/flatpak-vmtests/vm-${name}.sh /tmp/flatpak-vmtest/vmtests/ && "
        "chmod +x /tmp/flatpak-vmtest/vmtests/*.sh && "
        "chown -R testuser:users /tmp/flatpak-vmtest && "
        "mkdir -p /home/testuser/.local/share/flatpak && "
        "chown -R testuser:users /home/testuser/.local"
    )

    # Run the test as testuser with full output capture
    (rc, output) = machine.execute(
        "su - testuser -c '"
        "export FLATPAK=${pkgs.rust-flatpak-dev}/bin/flatpak; "
        "export WORK=/tmp/flatpak-vmtest; "
        "export HOME=/home/testuser; "
        "cd /tmp/flatpak-vmtest/vmtests && "
        "bash -x ./vm-${name}.sh 2>&1"
        "'",
        timeout=${toString testTimeout}
    )
    print(output)
    if rc != 0:
        raise Exception(f"Test vm-${name} failed with exit code {rc}")
  '';
}
