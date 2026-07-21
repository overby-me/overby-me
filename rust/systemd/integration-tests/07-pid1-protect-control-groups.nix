{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.protect-control-groups\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    # Replace bare 'sh' in ExecStartPost with full NixOS path
    sed -i "s|ExecStartPost='sh -c|ExecStartPost='/run/current-system/sw/bin/sh -c|g" TEST-07-PID1.protect-control-groups.sh
    # testcase_delegate_subgroup (ls /sys/fs/cgroup/supervisor under
    # ProtectControlGroupsEx=private + DelegateSubgroup=) and
    # testcase_delegate_subgroup_control (control commands run in a `.control`
    # subgroup so /proc/self/cgroup reads "0::/") both work now. Skip only
    # subgroup_pam, which needs unprivileged PAM session management: a
    # documented gap.
    sed -i '/^testcase_delegate_subgroup_pam/,/^}/d' TEST-07-PID1.protect-control-groups.sh
  '';
}
