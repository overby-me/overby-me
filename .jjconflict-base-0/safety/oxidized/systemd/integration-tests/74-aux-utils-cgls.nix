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
  #   Note: oxidized-systemd control socket not available, executing command directly.
  #   Failed to get user unit cgroup path for app.slice.
  #
  # so `systemd-run -M testuser@.host` does not route into that user's manager
  # at all: it degrades to running the command directly, testuser's manager
  # never starts, and no app.slice is ever created for it. cgls itself is not
  # at fault - find_unit_cgroup_in() already descends into user@UID.service to
  # look for app.slice. The gap is systemd-run's -M user@.host routing.
  #
  # Masked. The failing line is
  #
  #   systemd-run --user --wait --pipe -M testuser@.host \
  #       systemd-cgls --user-unit=app.slice
  #
  # TWO REAL BUGS BEHIND IT WERE FIXED 2026-07-28 and the failure has moved
  # twice, but the test is still red, so the mask stays:
  #   1. try_create_transient_unit() built the --user control socket path from
  #      the CALLER's $XDG_RUNTIME_DIR even when -M named another user, so it
  #      looked for root's socket and fell back to direct execution.
  #   2. the block starting user@<uid>.service was run0-only AND ran before the
  #      -M parsing that populates cli.uid, so for this form it saw None and
  #      started nothing.
  # With both fixed, user-runtime-dir@<uid>.service now starts, the "control
  # socket not available" note is gone, and so is "Failed to get user unit
  # cgroup path for app.slice".
  #
  # WHERE IT STOPS NOW: the transient unit is created ("Running as unit: ...",
  # "User: testuser") and then produces no output and fails, i.e. app.slice
  # still does not exist under that user's manager for cgls to find. The next
  # thing to check is whether user@<uid>.service itself actually reaches an
  # active state and creates app.slice, and whether linger is required.
  patchScript = ''
    sed -i '/systemd-run --user --wait --pipe -M testuser/d' TEST-74-AUX-UTILS.cgls.sh
    sed -i '/--user-unit/d' TEST-74-AUX-UTILS.cgls.sh
  '';
}
