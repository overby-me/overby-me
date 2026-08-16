{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.main-PID-change\\.sh$";
  };
  patchScript = ''    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
  '';
  # NOT A RUST BUG, measured 2026-07-28 against the C oracle. The subtest opens
  # with
  #
  #     MAINPID="${PPID:?}"
  #     test "$(systemctl show -P MainPID TEST-07-PID1.service)" -eq "$MAINPID"
  #
  # and dies there with "integer expected" because the property reads back
  # EMPTY. c-systemd-test-07-pid1-main-PID-change fails IDENTICALLY -- same exit
  # 1, same 143 traced lines, same command -- so real systemd cannot satisfy it
  # here either.
  #
  # The reason is the harness, not the manager: upstream runs its subtests from
  # a TEST-07-PID1.service unit, and this NixOS driver does not, so no such unit
  # exists to report a MainPID. TEST-07-PID1.service appears nowhere in the VM
  # log except the test's own command line.
  #
  # Note MainPID itself IS implemented, at control/unit_properties.rs:382, where
  # it is always inserted as either the pid or "0" -- an EMPTY value means the
  # service branch was never reached, i.e. the unit is unknown, not that the
  # property is missing. (Beware: units/unit_properties.rs does not exist;
  # grepping that path yields a false "absent".)
  #
  # Making this pass needs the harness to run subtests inside a
  # TEST-07-PID1.service unit. Do not "fix" rust for it.
}
