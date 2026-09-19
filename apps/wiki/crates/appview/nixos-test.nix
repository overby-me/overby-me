# End-to-end NixOS VM test for the stateful AppView (rewrite kickoff item 11's
# acceptance). Boots a real VM running `services.wiki-appview` behind the bundled
# Ferron reverse proxy and asserts the properties a persistent process needs but a
# plain derivation check cannot exercise: the service starts, `/healthz` reports
# liveness + DB-reachable (direct and through the reverse-proxy edge), the Turso
# StateDirectory survives a restart intact, structured JSON logs reach journald,
# and the systemd hardening is applied. Then the cutover's load, as an operator
# runs it on this very layout: `wiki-appview-import` stops the service, loads a
# (made-up) migrated wiki and its files into the private state directory, and
# the service serves them once started again.
#
# Run with: nix build .#checks.x86_64-linux.wiki-appview-e2e
{
  pkgs,
  wiki-appview,
}:
pkgs.testers.nixosTest {
  name = "wiki-appview-e2e";

  nodes.machine = {...}: {
    imports = [./nixos-module.nix];

    services.wiki-appview = {
      enable = true;
      package = wiki-appview;
      port = 8080;
      # The bundled Ferron proxy is on by default (proxyDomain = null -> a
      # plain-HTTP :80 catch-all forwarding to the AppView), which is exactly the
      # end-to-end edge path this test exercises.
      import = {
        extraction = "/etc/wiki-cutover/extraction.json";
        files = "/etc/wiki-cutover/files";
      };
    };

    # `src/import.rs` loads the same fixture in a unit test, so a change to what
    # an extraction looks like fails there first.
    environment.etc."wiki-cutover/extraction.json".source = ./fixtures/extraction.json;
    environment.etc."wiki-cutover/files".source = ./fixtures/files;

    # curl for the checks; the VM has no internet, which is fine: startup and
    # /healthz are offline (the firehose consumer connects best-effort with a
    # bounded timeout and only its status is reported, not required).
    environment.systemPackages = [pkgs.curl];
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("wiki-appview.service")
    machine.wait_for_open_port(8080)

    # /healthz: liveness + DB reachable (a real connection with the FK pragma).
    health = machine.succeed("curl -sf http://localhost:8080/healthz")
    assert '"ok":true' in health, f"healthz not ok: {health}"
    assert '"db":true' in health, f"db not reachable: {health}"

    # The same endpoint through the bundled Ferron reverse-proxy edge.
    machine.wait_for_unit("wiki-appview-proxy.service")
    machine.wait_for_open_port(80)
    proxied = machine.succeed("curl -sf http://localhost/healthz")
    assert '"ok":true' in proxied, f"healthz via ferron not ok: {proxied}"

    # The Turso file lives in the persistent StateDirectory.
    machine.succeed("test -d /var/lib/wiki-appview")
    machine.succeed("test -f /var/lib/wiki-appview/appview.db")

    # ...and it survives a service restart intact (a sentinel in the state dir
    # and the DB file both persist across the restart, and the DB is reachable
    # again). The schema init is guarded, so a persisted DB is reused, not reset.
    machine.succeed("touch /var/lib/wiki-appview/sentinel")
    machine.succeed("systemctl restart wiki-appview.service")
    machine.wait_for_unit("wiki-appview.service")
    machine.wait_for_open_port(8080)
    machine.succeed("test -f /var/lib/wiki-appview/sentinel")
    machine.succeed("test -f /var/lib/wiki-appview/appview.db")
    health2 = machine.succeed("curl -sf http://localhost:8080/healthz")
    assert '"db":true' in health2, f"db not reachable after restart: {health2}"

    # Structured JSON logs reach journald (server-side tracing, not the
    # browser-only logger): the startup line is present and JSON-shaped.
    logs = machine.succeed("journalctl -u wiki-appview.service --no-pager")
    assert "listening" in logs, "expected the startup log line in the journal"
    assert '"level":"INFO"' in logs, f"logs are not structured JSON: {logs[:300]}"

    # The systemd hardening the module declares is actually applied.
    unit = machine.succeed(
        "systemctl show wiki-appview.service "
        "--property=DynamicUser,ProtectSystem,NoNewPrivileges,PrivateTmp,ProtectHome"
    )
    assert "DynamicUser=yes" in unit, unit
    assert "ProtectSystem=strict" in unit, unit
    assert "NoNewPrivileges=yes" in unit, unit
    assert "PrivateTmp=yes" in unit, unit

    # The load. The service has run, so its datastore already has a home of its
    # own making, which has to make way for the one loaded.
    machine.succeed("systemctl start wiki-appview-import.service")
    machine.fail("systemctl is-active wiki-appview.service")
    machine.succeed("systemctl start wiki-appview.service")
    machine.wait_for_open_port(8080)
    xrpc = "http://localhost:8080/xrpc/com.example.wiki"
    page = machine.succeed(f"curl -sf '{xrpc}.getNode?path=open/minutes'")
    assert "The meeting was opened" in page, page
    agenda = machine.succeed("curl -sf http://localhost:8080/blob/0f000000-0000-4000-8000-0000000000f1")
    assert "an agenda" in agenda, agenda
    # The home came across closed, and stays closed to the signed out.
    machine.fail(f"curl -sf '{xrpc}.getNode?path='")

    # Run again, it finds everything there already.
    machine.succeed("systemctl start wiki-appview-import.service")
    loaded = machine.succeed("journalctl -u wiki-appview-import.service --no-pager")
    assert "1 copied" in loaded, loaded
    assert "0 copied, 1 here already" in loaded, loaded
  '';
}
