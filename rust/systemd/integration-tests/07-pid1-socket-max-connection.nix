{
  name = "07-PID1";
  extraPackages = pkgs: [pkgs.socat pkgs.util-linux];
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.socket-max-connection\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    # Replace bare commands in inline unit files with full NixOS paths
    sed -i 's|ExecStartPre=echo |ExecStartPre=/run/current-system/sw/bin/echo |g' TEST-07-PID1.socket-max-connection.sh
    sed -i 's|ExecStart=sleep |ExecStart=/run/current-system/sw/bin/sleep |g' TEST-07-PID1.socket-max-connection.sh
  '';
}
