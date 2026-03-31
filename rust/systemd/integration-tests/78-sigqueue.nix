{
  name = "78-SIGQUEUE";
  # Rewrite to avoid systemd-run/DynamicUser (not implemented).
  # Tests sigqueue signal delivery with blocked signals.
  patchScript = ''
    cat > TEST-78-SIGQUEUE.sh << 'TESTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    if ! env --block-signal=SIGUSR1 true 2>/dev/null; then
        echo "env tool too old, can't block signals, skipping test."
        touch /testok
        exit 0
    fi

    UNIT="test-sigqueue.service"
    cat > /run/systemd/system/$UNIT <<EOF
    [Service]
    Type=simple
    ExecStart=env --block-signal=SIGRTMIN+7 sleep infinity
    EOF

    systemctl start $UNIT
    sleep 1

    P=$(systemctl show -P MainPID $UNIT)
    # Record baseline SigQ (per-UID counter, not per-process)
    BEFORE=$(awk '/SigQ:/{split($2,a,"/"); print a[1]}' /proc/$P/status)

    systemctl kill --kill-whom=main --kill-value=4 --signal=SIGRTMIN+7 $UNIT
    systemctl kill --kill-whom=main --kill-value=4 --signal=SIGRTMIN+7 $UNIT
    systemctl kill --kill-whom=main --kill-value=7 --signal=SIGRTMIN+7 $UNIT
    systemctl kill --kill-whom=main --kill-value=16 --signal=SIGRTMIN+7 $UNIT
    systemctl kill --kill-whom=main --kill-value=32 --signal=SIGRTMIN+7 $UNIT
    systemctl kill --kill-whom=main --kill-value=16 --signal=SIGRTMIN+7 $UNIT

    AFTER=$(awk '/SigQ:/{split($2,a,"/"); print a[1]}' /proc/$P/status)
    DELTA=$((AFTER - BEFORE))
    echo "SigQ: before=$BEFORE after=$AFTER delta=$DELTA"
    test "$DELTA" -eq 6

    systemctl stop $UNIT
    rm /run/systemd/system/$UNIT

    touch /testok
    TESTEOF
    chmod +x TEST-78-SIGQUEUE.sh
  '';
}
