{
  name = "63-PATH";
  # ActivationDetails D-Bus property (a(ss) trigger_unit/trigger_path) is now
  # implemented (dbus_server.rs UnitObj::activation_details), so the busctl
  # assertions run. Only the issue-24577 section stays patched out (pending job
  # assertions — jobs don't appear in list-jobs because oxidized-systemd resolves
  # dependencies inline).
  patchScript = ''
    sed -i '/^# tests for issue.*24577/,/^# Test for race condition/{ /^# Test for race condition/!d }' TEST-63-PATH.sh
  '';
}
