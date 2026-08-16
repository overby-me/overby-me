{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-unit\\-files\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-unit-files.sh << 'LUFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-unit-files shows installed units"
    systemctl list-unit-files --no-pager | grep -q ".service"

    : "systemctl list-unit-files --type=service filters by type"
    systemctl list-unit-files --no-pager --type=service | grep -q ".service"

    : "systemctl list-unit-files --state=enabled filters to enabled units"
    LUF="luf-$RANDOM"
    cat > "/run/systemd/system/$LUF.service" << UEOF
    [Unit]
    Description=list-unit-files state filter test
    [Service]
    Type=oneshot
    ExecStart=true
    [Install]
    WantedBy=multi-user.target
    UEOF
    systemctl daemon-reload
    systemctl enable "$LUF.service"
    # the freshly-enabled unit appears under --state=enabled ...
    systemctl list-unit-files --no-pager --state=enabled | grep -q "$LUF.service"
    # ... and its own listed state is "enabled".
    systemctl list-unit-files --no-pager "$LUF.service" | grep -qw "enabled"
    systemctl disable "$LUF.service"
    rm -f "/run/systemd/system/$LUF.service"
    systemctl daemon-reload

    : "systemctl list-unit-files accepts a pattern"
    systemctl list-unit-files --no-pager "systemd-*" | grep -q "systemd-"
    LUFEOF
    chmod +x TEST-74-AUX-UTILS.list-unit-files.sh
  '';
}
