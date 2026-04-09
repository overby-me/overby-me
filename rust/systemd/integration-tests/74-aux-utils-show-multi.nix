{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-multi\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-multi.sh << 'SMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        systemctl stop show-a.service show-b.service 2>/dev/null
        rm -f /run/systemd/system/show-a.service /run/systemd/system/show-b.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl show -P works for multiple properties"
    cat > /run/systemd/system/show-a.service << EOF
    [Unit]
    Description=Show Test A
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    systemctl daemon-reload
    systemctl start show-a.service

    [[ "$(systemctl show -P Description show-a.service)" == "Show Test A" ]]
    [[ "$(systemctl show -P Type show-a.service)" == "oneshot" ]]
    [[ "$(systemctl show -P ActiveState show-a.service)" == "active" ]]
    [[ "$(systemctl show -P LoadState show-a.service)" == "loaded" ]]

    : "systemctl show for inactive unit shows correct state"
    cat > /run/systemd/system/show-b.service << EOF
    [Unit]
    Description=Show Test B
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    systemctl daemon-reload
    [[ "$(systemctl show -P ActiveState show-b.service)" == "inactive" ]]
    [[ "$(systemctl show -P Description show-b.service)" == "Show Test B" ]]
    SMEOF
    chmod +x TEST-74-AUX-UTILS.show-multi.sh
  '';
}
