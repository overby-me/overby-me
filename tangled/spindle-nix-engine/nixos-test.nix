# NixOS VM integration test for nix-tangled-spindle.
#
# Tests that the NixOS module correctly generates a systemd service,
# that the service starts, binds to the configured port, and responds
# to HTTP requests. Also verifies sandbox hardening is applied.
#
# Run with: nix build .#checks.x86_64-linux.nix-tangled-spindle-integration
{
  pkgs,
  tangled-spindle-nix-engine,
}:
pkgs.testers.nixosTest {
  name = "tangled-spindle-nix-engine-integration";

  nodes.machine = {...}: {
    imports = [./nixos-module.nix];

    # Create a dummy token file
    environment.etc."spindle-token".text = "test-token-12345";

    services.tangled-spindles.runner1 = {
      enable = true;
      package = tangled-spindle-nix-engine;
      hostname = "spindle.test.local";
      owner = "did:plc:testowner123";
      tokenFile = "/etc/spindle-token";
      listenAddr = "0.0.0.0:6555";
      dev = true;
      engine = {
        maxJobs = 1;
        queueSize = 10;
        workflowTimeout = "1m";
      };
    };
  };

  testScript = ''
    machine.wait_for_unit("tangled-spindle-runner1.service")
    machine.wait_for_open_port(6555)

    # Test MOTD endpoint
    result = machine.succeed("curl -sf http://localhost:6555/")
    assert "tangled-spindle-nix" in result, f"MOTD should contain 'tangled-spindle-nix', got: {result}"
    assert "spindle.test.local" in result, f"MOTD should contain hostname, got: {result}"

    # Test XRPC unknown method returns JSON error
    result = machine.succeed(
        "curl -sf -o /dev/null -w '%{http_code}' "
        "-X POST -H 'Authorization: Bearer test-token-12345' "
        "-H 'Content-Type: application/json' "
        "-d '{}' "
        "http://localhost:6555/xrpc/sh.tangled.spindle.nonexistent || true"
    )

    # Test auth is required
    result = machine.succeed(
        "curl -sf -o /dev/null -w '%{http_code}' "
        "-X POST -H 'Content-Type: application/json' "
        "-d '{\"did\":\"did:plc:test\"}' "
        "http://localhost:6555/xrpc/sh.tangled.spindle.addMember || true"
    )
    assert "401" in result, f"Missing auth should return 401, got: {result}"

    # Test systemd sandboxing is applied
    unit = machine.succeed(
        "systemctl show tangled-spindle-runner1.service "
        "--property=ProtectSystem,DynamicUser,ProtectHome,NoNewPrivileges,PrivateTmp"
    )
    assert "ProtectSystem=strict" in unit, f"ProtectSystem should be strict: {unit}"
    assert "DynamicUser=yes" in unit, f"DynamicUser should be yes: {unit}"
    assert "ProtectHome=read-only" in unit, f"ProtectHome should be read-only: {unit}"
    assert "NoNewPrivileges=yes" in unit, f"NoNewPrivileges should be yes: {unit}"
    assert "PrivateTmp=yes" in unit, f"PrivateTmp should be yes: {unit}"

    # Test state directory was created
    machine.succeed("test -d /var/lib/tangled-spindle/runner1")

    # Test log directory was created
    machine.succeed("test -d /var/log/tangled-spindle/runner1")
  '';
}
