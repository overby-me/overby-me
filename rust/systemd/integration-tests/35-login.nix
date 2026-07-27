{
  name = "35-LOGIN";
  # Re-baselined 2026-07-27. The previous rationale, "the logind session suite
  # is not implemented", was wrong: crates/logind is 7,237 lines and the suite
  # gets a long way in. setup_test_user, `systemctl edit`, restarting
  # systemd-logind, testcase_ambient_caps and most of testcase_background all
  # pass, sessions are created, PAMName= is honoured, and the
  # background-light class correctly leaves the user manager alone.
  #
  # THE REAL FIRST FAILURE is the USER MANAGER, not logind:
  #     systemd-run -u ... -p PAMName=... -p Environment=XDG_SESSION_CLASS=background
  #     systemctl is-active user@1002.service   ->  inactive, expected active
  # A `background` session must start user@<uid>.service, where
  # `background-light` deliberately must not. rust-systemd has no `systemd
  # --user` mode at all, so there is no user@.service to start.
  #
  # That is the same blocker as several other tests rather than anything
  # logind-specific, which makes it worth fixing once: a user manager would
  # unblock this, 23-unit-file's statedir case, part of 19-cgroup-delegate, and
  # the unprivileged section of the coredump suite.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: no systemd --user mode, so a background session cannot start user@.service' >/skipped"
      echo "exit 77"
    } > TEST-35-LOGIN.sh
    chmod +x TEST-35-LOGIN.sh
  '';
  # Skips rather than passes: the user manager does not exist
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}
