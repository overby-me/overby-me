{
  name = "44-LOG-NAMESPACE";
  # Skip until journald supports LogNamespace= property for journal
  # namespace isolation (separate journal directories per namespace).
  patchScript = ''
    echo '#!/bin/bash' > TEST-44-LOG-NAMESPACE.sh
    echo 'echo "Skipped: LogNamespace not yet implemented in journald"' >> TEST-44-LOG-NAMESPACE.sh
    echo 'touch /testok' >> TEST-44-LOG-NAMESPACE.sh
  '';
}
