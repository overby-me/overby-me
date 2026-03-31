{
  name = "26-SYSTEMCTL";
  # Skip sections requiring unimplemented features. Keep basic service
  # lifecycle, list commands, enable/disable, mask/unmask, and clean.
  patchScript = ''
    # Remove 'systemctl edit' tests (need EDITOR + script command)
    sed -i '/^EDITOR=/,/^# Argument help/{ /^# Argument help/!d }' TEST-26-SYSTEMCTL.sh
    # Remove global unit tests (--global flag not implemented)
    sed -i '/^# Test systemctl edit --global/,/^rm -f.*GLOBAL_MASKED_UNIT/d' TEST-26-SYSTEMCTL.sh
  '';
}
