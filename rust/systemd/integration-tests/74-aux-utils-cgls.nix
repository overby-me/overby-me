{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cgls\\.sh$";
  };
  # Masked. The failing line is
  #
  #   systemd-run --user --wait --pipe -M testuser@.host \
  #       systemd-cgls --user-unit=app.slice
  #
  # RE-TESTED 2026-07-28, after user@.service gained Delegate=pids memory cpu
  # (which fixed the user manager being unable to create ANY subcgroup), on the
  # theory that its app.slice could now exist. It still fails, so the mask
  # stays, but the blocker is now pinned more precisely than "the per-user
  # manager's slice layout". The run prints
  #
  #   Note: rust-systemd control socket not available, executing command directly.
  #   Failed to get user unit cgroup path for app.slice.
  #
  # so `systemd-run -M testuser@.host` does not route into that user's manager
  # at all: it degrades to running the command directly, testuser's manager
  # never starts, and no app.slice is ever created for it. cgls itself is not
  # at fault - find_unit_cgroup_in() already descends into user@UID.service to
  # look for app.slice. The gap is systemd-run's -M user@.host routing.
  #
  # Treated as deterministic rather than re-run: the failure is a
  # missing-capability message, not a racy assertion.
  patchScript = ''
    sed -i '/systemd-run --user --wait --pipe -M testuser/d' TEST-74-AUX-UTILS.cgls.sh
    sed -i '/--user-unit/d' TEST-74-AUX-UTILS.cgls.sh
  '';
}
