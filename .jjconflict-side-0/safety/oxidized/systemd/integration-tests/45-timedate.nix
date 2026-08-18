{
  name = "45-TIMEDATE";
  # testcase_ntp de-weakened (timedatectl set-ntp now goes through timedated
  # D-Bus, which emits PropertiesChanged). testcase_timesyncd still skipped
  # (needs networkd dummy-interface link-NTP setup).
  patchScript = ''
    sed -i '/^testcase_timesyncd/s/^testcase_/skipped_/' TEST-45-TIMEDATE.sh
  '';
}
