# Increment-4 A/B variant of 26-systemctl: run TEST-26-SYSTEMCTL.sh with PID 1
# booted through the job-graph run-queue drive (SYSTEMD_RS_JOB_GRAPH=1). Mirrors
# the base test's patchScript (same interactive-editor masks). Part of the
# EVENT-LOOP standing gate set; removed with the flag when the increment merges.
{
  name = "26-SYSTEMCTL";
  jobGraph = true;
  patchScript = ''
    # Interactive EDITOR tests need `script` for a TTY; script(1) hangs under PID 1.
    sed -i "/EDITOR=.*script -ec/d" TEST-26-SYSTEMCTL.sh
    sed -i '/^\[ ! -e.*override\.conf/d' TEST-26-SYSTEMCTL.sh
    sed -i '/^printf.*>"+4"$/d' TEST-26-SYSTEMCTL.sh
    sed -i '/^printf.*cmp.*\.d\/override\.conf"$/d' TEST-26-SYSTEMCTL.sh
  '';
}
