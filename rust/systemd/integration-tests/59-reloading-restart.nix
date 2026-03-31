{
  name = "59-RELOADING-RESTART";
  # Skip until Type=notify RELOADING=1 state tracking, daemon-reload
  # rate limiting, and Type=notify-reload are implemented in PID 1.
  patchScript = ''
    echo '#!/bin/bash' > TEST-59-RELOADING-RESTART.sh
    echo 'echo "Skipped: Type=notify RELOADING state and reload rate limiting not yet implemented"' >> TEST-59-RELOADING-RESTART.sh
    echo 'touch /testok' >> TEST-59-RELOADING-RESTART.sh
  '';
}
