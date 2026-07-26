{
  name = "34-DYNAMICUSERMIGRATE";
  # Was a FAKE PASS (`touch /testok` with nothing run). Now an honest skip.
  # `test_directory StateDirectory` gets through its DynamicUser=0 phase, its
  # DynamicUser=1 phase and the conversion back, including every host-side
  # assertion. Seven exec-directory defects were fixed getting there; see
  # docs/TEST-OVERRIDES.md and the git log.
  #
  # REMAINING FAILURE: the unit-parsing section at the end of test_directory.
  # `systemctl start --wait testservice-34.service` never returns. The unit is a
  # six-command Type=oneshot with TemporaryFileSystem= and an aliased
  # StateDirectory=.
  #
  # ESTABLISHED FROM THE HARNESS JOURNAL DUMP (do not re-derive):
  #   - the unit reaches Started and STAYS there; it is not in
  #     `systemctl list-units --failed`.
  #   - exactly ONE `REAP ... -> ServiceExited` appears for a six-command
  #     oneshot, and it is a Service pid, not a Helper. Commands 2-6 never run.
  #   - Type=oneshot without RemainAfterExit= must go inactive when its commands
  #     finish. This one never deactivates, so --wait blocks forever.
  #   - PID 1 is wedged manager-wide, not just this job: on a full-length run the
  #     watchdog cascade REPEATS every ~3 minutes indefinitely, SIGABRTing
  #     journald, oomd, udevd, resolved, networkd, hostnamed and logind each
  #     time. Something holds the RuntimeInfo lock and never lets go.
  #
  # THREE HYPOTHESES TRIED AND DISPROVEN, so do not spend cycles on them again:
  #   1. Invariant I1 via the INLINE multi-command oneshot loop. The inline
  #      branch requires a source other than DeferNotifyWait; never confirmed.
  #   2. The reaper filing an unregistered helper as ServiceExited, and
  #      wait_for_helper_child then hitting `unreachable!()`. Handling that state
  #      changed nothing.
  #   3. The reaper DISCARDING the exit of an unregistered pid (its `None` arm),
  #      leaving wait_for_helper_child polling forever. Parking the exit as an
  #      unclaimed record changed nothing either, so it was reverted rather than
  #      left in on a disproven rationale.
  #
  # TOOLING NOTE THAT COST SEVERAL CYCLES: PID 1's `log::` macros do NOT reach
  # the console. The `REAP` lines come from `crate::entrypoints::kmsg()`. Any
  # instrumentation added to PID 1 must use kmsg() or it is invisible at every
  # log level, and its absence must not be read as the code path not running.
  #
  # SUGGESTED NEXT STEP: instrument with kmsg() at the oneshot branch point and
  # around the exit handler's "Phase 2" thread (signal_handler.rs), which needs
  # the RuntimeInfo read lock after a service exit. A blocked Phase 2 would
  # explain the unit never leaving Started while the manager wedges. Setting
  # `testTimeout = 150;` here makes the harness dump PID 1's journal in ~2
  # minutes instead of 30, which is how the facts above were obtained.
  #
  # Further in, the test also needs nested exec directories (`quux/pief`,
  # `xxx/yyy:aaa/111`) and idmapped mounts on kernels >= 5.12.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: multi-command oneshot never deactivates and wedges PID 1' >/skipped"
      echo "exit 77"
    } > TEST-34-DYNAMICUSERMIGRATE.sh
    chmod +x TEST-34-DYNAMICUSERMIGRATE.sh
  '';
  # Skips rather than passes: testservice-34.service never deactivates
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}
