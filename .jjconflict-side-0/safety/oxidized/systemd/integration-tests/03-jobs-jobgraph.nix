# 03-jobs booted under SYSTEMD_RS_JOB_GRAPH=1 (increment-4 A/B). Exercises the
# job-graph boot producer plus the dispatcher drive end to end. The deferred-
# start completion race that used to stall this boot is fixed by the device-
# completion scan in drive_run_queue (Waiting Start jobs whose unit is already
# Started are retired even though .device units never fork-activate). Kept as a
# regression guard for the flag-on path.
{
  name = "03-JOBS";
  jobGraph = true;
}
