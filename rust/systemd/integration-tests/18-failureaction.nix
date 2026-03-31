{
  name = "18-FAILUREACTION";
  # Use upstream test with reboot/exit phases removed — the NixOS test
  # VM cannot survive SuccessAction=reboot (QEMU hard reset) or
  # FailureAction=exit (PID 1 exits, VM dies before /testok check).
  patchScript = ''
    sed -i '/^if ! test -f/,/^sleep infinity/d' TEST-18-FAILUREACTION.sh
    echo 'touch /testok' >> TEST-18-FAILUREACTION.sh
  '';
}
