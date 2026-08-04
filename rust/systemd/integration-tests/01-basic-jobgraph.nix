# Increment-4 A/B variant of 01-basic: boot PID 1 with SYSTEMD_RS_JOB_GRAPH=1 so
# the default target is brought up through the single dispatcher's run-queue
# drive (docs/EVENT-LOOP.md), which activates units on a bounded pool, instead of
# the fixpoint sweep. Same upstream TEST-01-BASIC.sh assertions; the point is
# that the boot closure still reaches the target under the job-graph path.
# Removed with the flag when the increment merges.
{
  name = "01-BASIC";
  jobGraph = true;
}
