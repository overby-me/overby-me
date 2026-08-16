{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.transient-unit-container\\.sh$";
  };
  # WHERE IT STOPS, re-measured 2026-07-28, correcting an earlier note here that
  # described this as a RootDirectory= output bug. It is not. The subtest boots a
  # whole nested systemd PID 1 inside a tmpfs root:
  #
  #     systemd-run --unit ... --wait -p RootDirectory=<tmpfs> \
  #         -p PrivatePIDs=yes -p PrivateUsersEx=full \
  #         -p ProtectControlGroupsEx=private -p Delegate=true \
  #         -p DelegateSubgroup=init.scope -p DelegateNamespaces=yes \
  #         -p BindPaths=<host>:<guest> ... \
  #         /usr/lib/systemd/systemd multi-user.target
  #
  # and only then reads back the file the container's own oneshot unit wrote
  # through the bind mount. The empty output file is the symptom of the
  # container never booting, not of a separate write path.
  #
  # DelegateNamespaces= appears in ZERO files under crates/, so it is silently
  # swallowed as an unknown transient property. That is the same blocker as
  # 07-pid1-private-bpf, rejected as deep. DelegateSubgroup=init.scope
  # additionally wants init.scope modelled as a real unit, which is the
  # separately deferred init.scope work.
  #
  # The failure surfaces in file_write_cleanup, so the tail of the log names the
  # cleanup rather than the defect; the real assertion is about ten traced lines
  # further back.
}
