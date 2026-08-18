{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.varlinkctl\\.sh$";
  };
  # WHERE IT STOPS, 2026-07-28, and it is no longer a varlinkctl gap:
  #
  #     varlinkctl list-registry | grep io.systemd.Manager
  #
  # list-registry itself works now and correctly reports an empty registry,
  # but nothing in rust ever POPULATES /run/systemd/varlink/registry, so PID 1
  # never advertises io.systemd.Manager there and the grep finds nothing.
  # Registering the manager's Varlink interface in that directory is a PID 1
  # feature, not a client one, so the remaining work has moved out of this
  # tool.
  #
  # Everything before it was fixed by walking the script forward, 153 -> 188
  # traced lines over four defects: no --help at all (it died on line one),
  # the unix:/exec: address prefixes being treated as literal paths, -j being
  # accepted and then ignored by list-interfaces and list-methods so their
  # output stayed human-readable and broke `| jq .`, and list-registry not
  # existing.
}
