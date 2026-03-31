{
  name = "38-FREEZER";
  # Enable all testcases except testcase_dbus_api (requires busctl).
  patchScript = ''
    # Skip testcases that use busctl D-Bus calls
    sed -i 's/^testcase_dbus_api/skipped_dbus_api/' TEST-38-FREEZER.sh
  '';
}
