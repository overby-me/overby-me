{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-remote\\.sh$";
  };
  testTimeout = 300;
  extraPackages = pkgs: [pkgs.openssl pkgs.curl];
  # WHERE IT STOPS, re-measured 2026-07-28 after the restart-ordering fix.
  #
  # An earlier version of this note blamed `unstarted_deps` in
  # units/unitset_manipulation/activate.rs. THAT WAS WRONG, twice over, and the
  # corrected story is worth keeping because both errors were easy to make:
  #
  #   * The error came from somewhere else entirely. "these related units did
  #     not have the expected state" is UnitOperationErrorReason::DependencyError,
  #     built ONLY in units/unit.rs (1582/1734/1832) from the bad_ids that
  #     Unit::state_transition_starting returns. `unstarted_deps` has just two
  #     callers and neither constructs it.
  #   * "Wants= is never enqueued" was too strong. collect_unit_start_subgraph
  #     expands via Dependencies::start_before_this(), which DOES include After=
  #     deps that are also Wants=/Requires=/BindsTo=/Upholds=, so plain
  #     `systemctl start` always pulled the target in. Only RESTART was broken.
  #
  # The real defect was ORDERING, and the correct code was already present but
  # unreachable: the Restart handlers called reactivate_unit FIRST, that returned
  # DependencyError while network-online.target was still NeverStarted, the `?`
  # propagated out, and the block right below it -- which starts exactly those
  # NeverStarted Wants=/Requires= deps -- never ran. Starting the dependencies
  # first fixes it. Three handlers shared the shape; the RestartNoBlock one only
  # logged its error instead of propagating, so it failed silently.
  #
  # THAT PART NOW WORKS: the DependencyError is gone and the test runs about
  # 2560 lines further, through the whole TLS upload round trip (25 entries
  # uploaded and imported).
  #
  # IT STILL FAILS, on a later and unrelated assertion:
  #
  #     timeout 10 bash -xec 'while [[ "$(systemctl show -P ActiveState \
  #         systemd-journal-upload)" != failed ]]; do sleep 1; done'
  #
  # ActiveState reads `inactive`, never `failed`. Upstream stops journal-remote
  # immediately before this section and points journal-upload at the server's own
  # key/cert as if they were client certs, so the upload has nothing to talk to
  # and must fail: upstream expects "failed with code 401", then
  # "Main process exited, code=exited, status=1/FAILURE" and, with a Restart=no
  # drop-in, the unit settling in `failed`. In our run journal-upload logs no
  # error at all and PID 1 records "Deactivated successfully".
  #
  # THE C ORACLE CANNOT SETTLE THIS ONE. c-systemd-test-04-journal-remote dies
  # EARLIER than we now do, at `systemctl restart systemd-journal-remote.socket`,
  # and never reaches the assertion -- so it says nothing about it either way.
  # (Do not read that as a pass for us; it only means real systemd stops sooner
  # in this harness.)
  #
  # STILL OPEN, and NOT yet attributed: whether journal-upload itself exits 0
  # when it cannot reach the server, or whether the exit status is recorded but
  # mis-mapped to `inactive` instead of `failed`. Nothing in the VM log shows the
  # process's exit code, so do not guess. Note 59-RELOADING-RESTART is stuck on a
  # suspiciously similar question (ExecMainStatus empty for a stopped unit), so
  # check whether one cause explains both.
}
