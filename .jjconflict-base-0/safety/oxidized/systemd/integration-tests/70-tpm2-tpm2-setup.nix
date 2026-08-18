{
  name = "70-TPM2";
  # systemd-tpm2-setup needs a TPM2 device; attach a software TPM to the VM.
  enableTpm = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.tpm2-setup\\.sh$";
  };
}
