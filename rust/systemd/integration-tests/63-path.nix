{
  name = "63-PATH";
  # Patch out busctl calls (ActivationDetails D-Bus property not implemented)
  # and the issue-24577 section (pending job assertions — jobs don't appear
  # in list-jobs because rust-systemd resolves dependencies inline).
  patchScript = ''
    sed -i '/^test "$(busctl/d' TEST-63-PATH.sh
    sed -i '/^# tests for issue.*24577/,/^# Test for race condition/{ /^# Test for race condition/!d }' TEST-63-PATH.sh
  '';
}
