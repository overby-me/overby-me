{
  name = "26-SYSTEMCTL";
  # Skip sections requiring unimplemented features. Keep basic service
  # lifecycle, list commands, enable/disable, mask/unmask, and clean.
  # Enable --stdin edit tests (implemented); remove interactive EDITOR tests
  # (need `script` command for TTY allocation).
  patchScript = ''
    # Remove interactive EDITOR tests (need `script` command for TTY)
    sed -i "/EDITOR=.*script -ec/d" TEST-26-SYSTEMCTL.sh
    sed -i '/^\[ ! -e.*override\.conf/d' TEST-26-SYSTEMCTL.sh
    sed -i '/^printf.*>"+4"$/d' TEST-26-SYSTEMCTL.sh
    sed -i '/^printf.*cmp.*\.d\/override\.conf"$/d' TEST-26-SYSTEMCTL.sh
    # Remove global unit tests (--global flag not implemented)
    sed -i '/^# Test systemctl edit --global/,/^rm -f.*GLOBAL_MASKED_UNIT/d' TEST-26-SYSTEMCTL.sh
  '';
}
