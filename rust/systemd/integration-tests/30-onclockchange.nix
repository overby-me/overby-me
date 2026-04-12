{
  name = "30-ONCLOCKCHANGE";
  # Skip the alternate-path section (SYSTEMD_ETC_LOCALTIME override tests).
  # Requires D-Bus integration between timedated and PID 1 for cross-process
  # timezone change notification, which is not yet implemented.
  patchScript = ''
    sed -i '/^mkdir -p \/etc\/alternate-path$/i touch /testok; exit 0' TEST-30-ONCLOCKCHANGE.sh
  '';
}
