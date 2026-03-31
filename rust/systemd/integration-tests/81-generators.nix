{
  name = "81-GENERATORS";
  # Use upstream subtests. These test generator binaries directly
  # (not through PID 1) so they don't need D-Bus or other PID 1 features.
  patchScript = ''
    # Remove environment-d-generator subtest: it tests a user-session
    # generator that requires XDG_CONFIG_DIRS and user-level paths
    # which differ significantly on NixOS.
    rm -f TEST-81-GENERATORS.environment-d-generator.sh
  '';
}
