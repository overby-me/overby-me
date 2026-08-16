{
  name = "10-MOUNT";
  # NOTE (de-skip experiment): upstream 10-MOUNT depends on rust-systemd
  # detecting manually-issued mount(8) operations via /proc/self/mountinfo and
  # synthesising dynamic .mount units. Baselining un-skipped to find the real
  # first failure (per the #72 lesson) before implementing the mountinfo monitor.
}
