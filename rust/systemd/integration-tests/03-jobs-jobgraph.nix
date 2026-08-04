# Increment-4 A/B variant of 03-jobs: run TEST-03-JOBS.sh with PID 1 booted
# through the job-graph run-queue drive (SYSTEMD_RS_JOB_GRAPH=1). Mirrors the
# base test's patchScript. Part of the EVENT-LOOP standing gate set; removed
# with the flag when the increment merges.
{
  name = "03-JOBS";
  jobGraph = true;
  patchScript = ''
    # Fix upstream typo: propagatesstopto → propagatestopto
    sed -i 's/propagatesstopto-indirect/propagatestopto-indirect/g' TEST-03-JOBS.sh
  '';
}
