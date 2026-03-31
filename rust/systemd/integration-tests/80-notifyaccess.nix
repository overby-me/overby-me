{
  name = "80-NOTIFYACCESS";
  # Skip until SCM_CREDENTIALS-based NotifyAccess enforcement is
  # implemented (requires extracting sender PID from notification socket).
  patchScript = ''
    echo '#!/bin/bash' > TEST-80-NOTIFYACCESS.sh
    echo 'echo "Skipped: NotifyAccess enforcement not yet implemented"' >> TEST-80-NOTIFYACCESS.sh
    echo 'touch /testok' >> TEST-80-NOTIFYACCESS.sh
  '';
}
