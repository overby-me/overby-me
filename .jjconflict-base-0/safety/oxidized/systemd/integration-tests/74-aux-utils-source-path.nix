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

    : "SourcePath is empty for a normal (non-generated) unit"
    SP="$(systemctl show -P SourcePath systemd-journald.service)"
    # A regular unit file has no SourcePath (that field is only set for units
    # synthesised from another source, e.g. a generator or /etc/fstab). It must
    # not leak the fragment path.
    [[ -z "$SP" ]]

    : "Id property for well-known unit"
    ID="$(systemctl show -P Id systemd-journald.service)"
    [[ "$ID" == "systemd-journald.service" ]]
    SPEOF
    chmod +x TEST-74-AUX-UTILS.source-path.sh
  '';
}
