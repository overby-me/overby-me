{
  name = "45-TIMEDATE";
  # Skip NTP and timesyncd testcases (busctl monitor signal parsing).
  patchScript = ''
    sed -i '/^testcase_ntp/s/^testcase_/skipped_/' TEST-45-TIMEDATE.sh
    sed -i '/^testcase_timesyncd/s/^testcase_/skipped_/' TEST-45-TIMEDATE.sh
  '';
}
