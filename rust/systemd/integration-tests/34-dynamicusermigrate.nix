{
  name = "34-DYNAMICUSERMIGRATE";
  # Skip until StateDirectory= alias syntax (e.g. zzz:yyy) and
  # DynamicUser= directory migration are implemented in PID 1.
  patchScript = ''
    echo '#!/bin/bash' > TEST-34-DYNAMICUSERMIGRATE.sh
    echo 'echo "Skipped: StateDirectory alias and DynamicUser migration not yet implemented"' >> TEST-34-DYNAMICUSERMIGRATE.sh
    echo 'touch /testok' >> TEST-34-DYNAMICUSERMIGRATE.sh
  '';
}
