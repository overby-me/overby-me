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
    # subgroup so /proc/self/cgroup reads "0::/") both work now.
    #
    # subgroup_pam used to be deleted here as needing unprivileged PAM session
    # management. Unprivileged user-manager support has since landed
    # (user@.service declares Delegate= so the manager's cgroup is chown'd to
    # the user, run_pam_session() calls pam_putenv, run0 sets
    # PAMName=systemd-run0), and the whole testcase now runs and passes, so the
    # deletion is gone. Verified over two runs, with subgroup_pam visible in
    # the trace rather than inferred from a green.
    #
    # FLAKE seen while checking this, worth knowing before blaming a change:
    # testcase_delegate_subgroup_control failed once on its assert_eq of
    # /proc/self/cgroup against 0::/ , having read it back empty, and passed on
    # the next three runs. (The literal empty-string argument is not quoted
    # here: a doubled single quote would end this Nix string.) It
    # looked at first like removing the deletion had broken it, but the
    # testcase order was identical in the passing and failing runs and
    # subgroup_pam had not executed at all in either (it sorts last, and the
    # failing run aborted before reaching it). Re-run before diagnosing.
  '';
}
