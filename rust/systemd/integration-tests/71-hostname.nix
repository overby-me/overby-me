{
  name = "71-HOSTNAME";
  # Skip nss-myhostname testcase: the module is present but doesn't
  # resolve *.localhost subdomains (foo.localhost) in this VM config.
  # This is a C-library systemd feature, not a rust-systemd concern.
  patchScript = ''
    sed -i '/^testcase_nss-myhostname/s/^testcase_/skipped_/' TEST-71-HOSTNAME.sh
  '';
}
