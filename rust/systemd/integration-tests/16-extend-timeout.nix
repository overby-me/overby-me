{
  name = "16-EXTEND-TIMEOUT";
  # Upstream tests rely on a meson-generated wrapper service whose Wants=
  # pulls in the success-* / fail-* helper services on boot.  Our nix VM
  # doesn't have that scaffolding, so the test would `wait_for` on services
  # that never started.  Patch the script to systemctl-start them first.
  patchScript = ''
    sed -i '/^TESTLOG=/a\
\
# rust-systemd: explicitly start helper services that the meson-generated\
# upstream test runner would Wants= into the boot transaction.\
systemctl start --no-block success-all.service success-start.service success-runtime.service success-stop.service || :\
systemctl start --no-block fail-start.service fail-stop.service fail-runtime.service || :' \
        TEST-16-EXTEND-TIMEOUT.sh
  '';
}
