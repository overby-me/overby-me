{
  name = "05-RLIMITS";
  # Patch rlimit subtest: replace --pty (-t) with --pipe (-P) since TTY
  # allocation is not available in the test VM.
  patchScript = ''
    sed -i 's/systemd-run --wait -t/systemd-run --wait --pipe/' TEST-05-RLIMITS.rlimit.sh
  '';
}
