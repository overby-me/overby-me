{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.path-makedirectory\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.path-makedirectory.sh << 'MDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="makedir-$RANDOM"
    WATCHDIR="/run/makedir-test-$RANDOM"
    rm -rf "$WATCHDIR"

    at_exit() {
        set +e
        systemctl stop "$UNIT.path" "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.path" "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.path" "/run/systemd/system/$UNIT.service"
        rm -rf "$WATCHDIR"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # A .path watching a directory that does not exist yet, with MakeDirectory=yes:
    # starting the path unit must create the directory (with DirectoryMode=) so the
    # watch can be established directly.
    cat > "/run/systemd/system/$UNIT.path" << EOF
    [Path]
    DirectoryNotEmpty=$WATCHDIR
    MakeDirectory=yes
    DirectoryMode=0700
    EOF
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    systemctl daemon-reload

    : "Starting the .path with MakeDirectory=yes creates the watched directory"
    systemctl start "$UNIT.path"
    for _ in $(seq 1 50); do
        [[ -d "$WATCHDIR" ]] && break
        sleep 0.2
    done
    test -d "$WATCHDIR"

    : "The created directory has DirectoryMode=0700"
    test "$(stat -c %a "$WATCHDIR")" = "700"
    MDEOF
  '';
}
