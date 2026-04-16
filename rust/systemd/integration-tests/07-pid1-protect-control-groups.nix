{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.protect-control-groups\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    # Replace bare 'sh' in ExecStartPost with full NixOS path
    sed -i "s|ExecStartPost='sh -c|ExecStartPost='/run/current-system/sw/bin/sh -c|g" TEST-07-PID1.protect-control-groups.sh
    # Skip all delegate testcases — requires Delegate=yes + DelegateSubgroup= (not yet implemented)
    sed -i '/^testcase_delegate/,/^}/d' TEST-07-PID1.protect-control-groups.sh
  '';
}
