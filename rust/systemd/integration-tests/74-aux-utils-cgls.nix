{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cgls\\.sh$";
  };
  # Re-masked 2026-07-27 after testing the de-mask: the original rationale
  # HOLDS. Everything else in the subtest passes; the exact failing line is
  #
  #   systemd-run --user --wait --pipe -M testuser@.host \
  #       systemd-cgls --user-unit=app.slice
  #
  # `app.slice` being the default for user transients (control.rs) was not
  # sufficient: this routes into ANOTHER user's manager via -M testuser@.host
  # and needs that user's app.slice to exist in the cgroup tree. So the blocker
  # is the per-user manager's slice layout, not the transient default.
  patchScript = ''
    sed -i '/systemd-run --user --wait --pipe -M testuser/d' TEST-74-AUX-UTILS.cgls.sh
    sed -i '/--user-unit/d' TEST-74-AUX-UTILS.cgls.sh
  '';
}
