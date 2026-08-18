# Increment-4 A/B variant of 15-dropin: run TEST-15-DROPIN.sh with PID 1 booted
# through the job-graph run-queue drive (SYSTEMD_RS_JOB_GRAPH=1). Mirrors the
# base test's patchScript. Part of the EVENT-LOOP standing gate set; removed
# with the flag when the increment merges.
{
  name = "15-DROPIN";
  jobGraph = true;
  patchScript = ''
    # Replace bare `sleep` in inline unit files with full NixOS path.
    # oxidized-systemd respects the service's `Environment=PATH=` + the
    # PID-1-inherited PATH, but NixOS's upstream systemd compiled-in
    # DEFAULT_PATH_NORMAL doesn't include /run/current-system/sw/bin
    # — so the exec helper falls back to that built-in path and fails
    # to resolve bare command names in inline units.
    sed -i 's|ExecStart=sleep |ExecStart=/run/current-system/sw/bin/sleep |g' \
      TEST-15-DROPIN.sh
  '';
}
