{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-jobs\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-jobs.sh << 'LJEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-jobs reports the job queue"
    OUT="$(systemctl list-jobs --no-pager 2>&1)"
    # With no pending jobs, list-jobs prints its JOB header and/or a
    # 'No jobs' notice; either way the output must be well-formed.
    grep -qiE "JOB|No jobs" <<<"$OUT"
    LJEOF
    chmod +x TEST-74-AUX-UTILS.list-jobs.sh
  '';
}
