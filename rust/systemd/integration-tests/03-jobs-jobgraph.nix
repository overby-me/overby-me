# Increment-4 A/B variant of 03-jobs, re-added to diagnose the intermittent boot
# stall under the job-graph drive (SYSTEMD_RS_JOB_GRAPH=1). The boot producer now
# logs the still-pending jobs via kmsg every ~5s ("JOB-GRAPH pending ..."), so a
# stalled run names the stuck units. Mirrors the base test's patchScript.
{
  name = "03-JOBS";
  jobGraph = true;
  patchScript = ''
    # Fix upstream typo: propagatesstopto → propagatestopto
    sed -i 's/propagatesstopto-indirect/propagatestopto-indirect/g' TEST-03-JOBS.sh
  '';
}
