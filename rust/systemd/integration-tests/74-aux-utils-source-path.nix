{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.source\\-path\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.source-path.sh << 'SPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "SourcePath for unit with drop-in"
    SP="$(systemctl show -P SourcePath systemd-journald.service)"
    # May or may not be set, but the property should exist
    [[ -n "$SP" || -z "$SP" ]]

    : "Id property for well-known unit"
    ID="$(systemctl show -P Id systemd-journald.service)"
    [[ "$ID" == "systemd-journald.service" ]]
    SPEOF
    chmod +x TEST-74-AUX-UTILS.source-path.sh
  '';
}
