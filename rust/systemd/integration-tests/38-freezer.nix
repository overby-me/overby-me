{
  name = "38-FREEZER";
  # testcase_dbus_api now runs: the Unit Freeze()/Thaw() and Manager
  # FreezeUnit()/ThawUnit() D-Bus methods are implemented (dbus_server.rs),
  # wired to the same cgroup-freezer path as `systemctl freeze/thaw`.
  patchScript = "";
}
