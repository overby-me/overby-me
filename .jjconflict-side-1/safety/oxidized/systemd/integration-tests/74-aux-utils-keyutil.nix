{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.keyutil\\.sh$";
  };
  # The keyutil subtest drives systemd-keyutil against openssl-generated
  # certificates and verifies its PKCS7 signatures with `openssl smime`, so the
  # test VM needs the openssl CLI on PATH (matching the upstream test env).
  extraPackages = pkgs: [pkgs.openssl];
}
