{
  name = "15-DROPIN";
  patchScript = ''
    # Replace bare `sleep` in inline unit files with full NixOS path.
    # rust-systemd respects the service's `Environment=PATH=` + the
    # PID-1-inherited PATH, but NixOS's upstream systemd compiled-in
    # DEFAULT_PATH_NORMAL doesn't include /run/current-system/sw/bin
    # — so the exec helper falls back to that built-in path and fails
    # to resolve bare command names in inline units.
    sed -i 's|ExecStart=sleep |ExecStart=/run/current-system/sw/bin/sleep |g' \
      TEST-15-DROPIN.sh
  '';
}
