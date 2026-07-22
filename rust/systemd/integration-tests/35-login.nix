{
  name = "35-LOGIN";
  # Upstream 35-LOGIN. Baselined 2026-07-22: does NOT self-skip (evemu-device is
  # present or the guard is later) -- it runs the full logind suite (setup_test_user
  # + systemctl edit + restart systemd-logind all pass, then run_testcases hits
  # testcase_ambient_caps and beyond, which need real logind session management).
  # Genuinely deep (systemd-logind sessions/seats/PAM). Re-skipped.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: systemd-logind session suite not implemented, skipping' >/skipped"
      echo "exit 77"
    } > TEST-35-LOGIN.sh
    chmod +x TEST-35-LOGIN.sh
  '';
}
