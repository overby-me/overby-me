{
  name = "80-NOTIFYACCESS";
  # Custom test: verify NotifyAccess= enforcement via SCM_CREDENTIALS, the
  # status-error triad (ERRNO=/BUSERROR=/VARLINKERROR=), the Type=notify-reload
  # reload substates (reload-signal/reload-notify) with ReloadResult=timeout,
  # and (2026-08-05) the fd-store pinning lifecycle from the upstream fdstore
  # section: FileDescriptorStorePreserve=yes vs restart, NFileDescriptorStore,
  # survival across restart, release on a full stop unless pinned, the
  # SubState=dead-resources-pinned pinned-dead state, and
  # `systemctl clean --what=fdstore`. Still omitted vs upstream: the
  # `systemd-analyze fdstore --json=short` exact-format assertion (the analyze
  # verb's JSON layout is a separate surface).
  patchScript = ''
        cat > TEST-80-NOTIFYACCESS.sh << 'TESTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop testnotify-main.service 2>/dev/null
        systemctl stop testnotify-all.service 2>/dev/null
        systemctl stop testnotify-none.service 2>/dev/null
        systemctl stop testnotify-exec.service 2>/dev/null
        systemctl stop testnotify-status.service 2>/dev/null
        systemctl stop testnotify-reload-timeout.service 2>/dev/null
        systemctl stop testnotify-reload-ok.service 2>/dev/null
        systemctl stop fdstore-pin.service fdstore-nopin.service 2>/dev/null
        systemctl clean fdstore-pin.service --what=fdstore 2>/dev/null
        rm -f /run/systemd/system/testnotify-*.service
        rm -f /run/systemd/system/fdstore-pin.service /run/systemd/system/fdstore-nopin.service /run/systemd/system/fdstore-pin.target
        rm -f /run/fdstore-pin.sh /tmp/fdstore-invoked.* /tmp/fdstore-data.*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Write all service files upfront and do a single daemon-reload
    cat > /run/systemd/system/testnotify-all.service <<EOF
    [Service]
    Type=notify
    NotifyAccess=all
    ExecStart=/usr/bin/bash -c 'systemd-notify --ready && sleep infinity'
    EOF

    cat > /run/systemd/system/testnotify-main.service <<EOF
    [Service]
    Type=notify
    NotifyAccess=main
    ExecStart=/usr/bin/bash -c 'systemd-notify --ready && sleep infinity'
    EOF

    cat > /run/systemd/system/testnotify-none.service <<EOF
    [Service]
    Type=notify
    NotifyAccess=none
    TimeoutStartSec=3
    ExecStart=/usr/bin/bash -c 'systemd-notify --ready && sleep infinity'
    EOF

    cat > /run/systemd/system/testnotify-exec.service <<EOF
    [Service]
    Type=notify
    NotifyAccess=exec
    ExecStart=/usr/bin/bash -c 'systemd-notify --ready && sleep infinity'
    EOF

    systemctl daemon-reload

    : "NotifyAccess=all — any process can send READY=1"
    systemctl start testnotify-all.service
    timeout 30 bash -c 'while [ "$(systemctl is-active testnotify-all.service)" != active ]; do sleep 0.5; done'
    assert_eq "$(systemctl is-active testnotify-all.service)" "active"
    systemctl stop testnotify-all.service
    sleep 1

    : "NotifyAccess=main — main PID process group can send READY=1"
    systemctl start testnotify-main.service
    timeout 30 bash -c 'while [ "$(systemctl is-active testnotify-main.service)" != active ]; do sleep 0.5; done'
    assert_eq "$(systemctl is-active testnotify-main.service)" "active"
    systemctl stop testnotify-main.service
    sleep 1

    : "NotifyAccess=exec — service process group can send READY=1"
    systemctl start testnotify-exec.service
    timeout 30 bash -c 'while [ "$(systemctl is-active testnotify-exec.service)" != active ]; do sleep 0.5; done'
    assert_eq "$(systemctl is-active testnotify-exec.service)" "active"
    systemctl stop testnotify-exec.service
    sleep 1

    : "NotifyAccess=none — READY=1 rejected, service times out"
    (! systemctl start testnotify-none.service)
    # The start returns on timeout, but the transition to the failed state is
    # asynchronous; poll for it (as the other cases poll for active) rather than
    # racing the immediate is-failed read.
    timeout 10 bash -c 'while [ "$(systemctl is-failed testnotify-none.service)" != failed ]; do sleep 0.5; done'
    assert_eq "$(systemctl is-failed testnotify-none.service)" "failed"
    systemctl reset-failed testnotify-none.service 2>/dev/null || true

    : "Status-error triad — ERRNO=/BUSERROR=/VARLINKERROR= reported via systemctl show"
    cat > /run/systemd/system/testnotify-status.service <<EOF
    [Service]
    Type=notify
    NotifyAccess=all
    ExecStart=/usr/bin/bash -c 'systemd-notify --ready ERRNO=1 BUSERROR=org.freedesktop.DBus.Error.InvalidArgs VARLINKERROR=org.varlink.service.InvalidParameter && sleep infinity'
    EOF
    systemctl daemon-reload
    systemctl start testnotify-status.service
    timeout 30 bash -c 'while [ "$(systemctl is-active testnotify-status.service)" != active ]; do sleep 0.5; done'
    assert_eq "$(systemctl show testnotify-status.service -P StatusErrno)" "1"
    assert_eq "$(systemctl show testnotify-status.service -P StatusBusError)" "org.freedesktop.DBus.Error.InvalidArgs"
    assert_eq "$(systemctl show testnotify-status.service -P StatusVarlinkError)" "org.varlink.service.InvalidParameter"
    systemctl stop testnotify-status.service

    : "Type=notify-reload — SubState reload lifecycle and ReloadResult=timeout"
    cat > /run/systemd/system/testnotify-reload-timeout.service <<EOF
    [Service]
    Type=notify-reload
    NotifyAccess=all
    TimeoutStartSec=5
    ExecStart=/usr/bin/bash -c 'trap "systemd-notify --reloading" SIGHUP; systemd-notify --ready; while :; do sleep 0.2; done'
    EOF
    cat > /run/systemd/system/testnotify-reload-ok.service <<EOF
    [Service]
    Type=notify-reload
    NotifyAccess=all
    TimeoutStartSec=30
    ExecStart=/usr/bin/bash -c 'trap "systemd-notify --reloading; systemd-notify --ready" SIGHUP; systemd-notify --ready; while :; do sleep 0.2; done'
    EOF
    systemctl daemon-reload
    systemctl start testnotify-reload-timeout.service testnotify-reload-ok.service
    timeout 30 bash -c 'while [ "$(systemctl is-active testnotify-reload-timeout.service)" != active ]; do sleep 0.5; done'
    timeout 30 bash -c 'while [ "$(systemctl is-active testnotify-reload-ok.service)" != active ]; do sleep 0.5; done'

    # Timeout path: RELOADING=1 arrives but READY=1 never does, so after
    # TimeoutStartSec the reload gives up: SubState returns to running and
    # ReloadResult becomes timeout.
    systemctl reload --no-block testnotify-reload-timeout.service
    # Poll briefly for the reload phase to appear. The property read is
    # non-blocking, so a single contended sample can momentarily miss it, and
    # RELOADING=1 may arrive between samples (reload-signal -> reload-notify).
    found=0
    n=0
    while [ "$n" -lt 20 ]; do
        sub=$(systemctl show testnotify-reload-timeout.service -P SubState)
        case "$sub" in reload-signal|reload-notify) found=1; break ;; esac
        n=$((n + 1))
        sleep 0.2
    done
    echo "reload-timeout SubState during reload = $sub (want reload-signal or reload-notify)"
    test "$found" = 1
    timeout 25 bash -c 'while [ "$(systemctl show testnotify-reload-timeout.service -P SubState)" != running ]; do sleep 1; done'
    assert_eq "$(systemctl show testnotify-reload-timeout.service -P ReloadResult)" "timeout"

    # Success path: the reload signal handler sends RELOADING=1 then READY=1,
    # so the reload completes with SubState running and ReloadResult success.
    systemctl reload --no-block testnotify-reload-ok.service
    timeout 25 bash -c 'while [ "$(systemctl show testnotify-reload-ok.service -P SubState)" != running ]; do sleep 0.5; done'
    assert_eq "$(systemctl show testnotify-reload-ok.service -P ReloadResult)" "success"

    systemctl stop testnotify-reload-timeout.service testnotify-reload-ok.service

    : "fd store pinning lifecycle (FileDescriptorStorePreserve=yes vs restart)"
    # Upstream fdstore-pin.sh: called three times per service (start, restart,
    # stop+start). It stores an fd via systemd-notify --fd on the first run and
    # asserts LISTEN_FDS re-passing (0 first, 1 on restart / pinned re-start).
    rm -f /tmp/fdstore-invoked.* /tmp/fdstore-data.*
    cat > /run/fdstore-pin.sh <<'SCRIPTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail
    PINNED="$1"
    COUNTER="/tmp/fdstore-invoked.$PINNED"
    FILE="/tmp/fdstore-data.$PINNED"
    if [ -e "$COUNTER" ] ; then
        read -r N < "$COUNTER"
    else
        N=0
    fi
    echo "Invocation #$N with PINNED=$PINNED."
    if [ "$N" -eq 0 ] ; then
        test "''${LISTEN_FDS:-0}" -eq 0
        test ! -e "$FILE"
        echo waldi > "$FILE"
        systemd-notify --fd=3 --fdname="fd-$N-$PINNED" 3< "$FILE"
    elif [ "$N" -eq 1 ] || { [ "$N" -eq 2 ] && [ "$PINNED" -eq 1 ]; } ; then
        test "''${LISTEN_FDS:-0}" -eq 1
        read -r word < /proc/self/fd/3
        test "$word" = "waldi"
    else
        test "''${LISTEN_FDS:-0}" -eq 0
        test -e "$FILE"
    fi
    if [ "$N" -ge 2 ] ; then
        rm "$COUNTER" "$FILE"
    else
        echo $((N + 1)) > "$COUNTER"
    fi
    systemd-notify --ready --status="Ready"
    exec sleep infinity
    SCRIPTEOF
    chmod +x /run/fdstore-pin.sh

    cat > /run/systemd/system/fdstore-pin.service <<EOF
    [Service]
    Type=notify
    NotifyAccess=all
    FileDescriptorStoreMax=10
    FileDescriptorStorePreserve=yes
    ExecStart=/usr/bin/bash /run/fdstore-pin.sh 1
    EOF
    cat > /run/systemd/system/fdstore-nopin.service <<EOF
    [Service]
    Type=notify
    NotifyAccess=all
    FileDescriptorStoreMax=10
    FileDescriptorStorePreserve=restart
    ExecStart=/usr/bin/bash /run/fdstore-pin.sh 0
    EOF
    cat > /run/systemd/system/fdstore-pin.target <<EOF
    [Unit]
    After=fdstore-pin.service fdstore-nopin.service
    Wants=fdstore-pin.service fdstore-nopin.service
    EOF
    systemctl daemon-reload

    systemctl start fdstore-pin.target
    timeout 30 bash -c 'while [ "$(systemctl is-active fdstore-pin.service)" != active ]; do sleep 0.5; done'
    timeout 30 bash -c 'while [ "$(systemctl is-active fdstore-nopin.service)" != active ]; do sleep 0.5; done'
    assert_eq "$(systemctl show fdstore-pin.service -P FileDescriptorStorePreserve)" yes
    assert_eq "$(systemctl show fdstore-nopin.service -P FileDescriptorStorePreserve)" restart
    assert_eq "$(systemctl show fdstore-pin.service -P NFileDescriptorStore)" 1
    assert_eq "$(systemctl show fdstore-nopin.service -P NFileDescriptorStore)" 1

    # The fd store survives a restart for both pin and nopin.
    systemctl restart fdstore-pin.service fdstore-nopin.service
    timeout 30 bash -c 'while [ "$(systemctl is-active fdstore-pin.service)" != active ]; do sleep 0.5; done'
    timeout 30 bash -c 'while [ "$(systemctl is-active fdstore-nopin.service)" != active ]; do sleep 0.5; done'
    assert_eq "$(systemctl show fdstore-pin.service -P NFileDescriptorStore)" 1
    assert_eq "$(systemctl show fdstore-nopin.service -P NFileDescriptorStore)" 1

    # A full stop keeps the store only when pinned (=yes); =restart drops it.
    systemctl stop fdstore-pin.service fdstore-nopin.service
    assert_eq "$(systemctl show fdstore-pin.service -P NFileDescriptorStore)" 1
    assert_eq "$(systemctl show fdstore-nopin.service -P NFileDescriptorStore)" 0
    assert_eq "$(systemctl show fdstore-pin.service -P SubState)" dead-resources-pinned
    assert_eq "$(systemctl show fdstore-nopin.service -P SubState)" dead

    systemctl start fdstore-pin.service fdstore-nopin.service
    timeout 30 bash -c 'while [ "$(systemctl is-active fdstore-pin.service)" != active ]; do sleep 0.5; done'
    timeout 30 bash -c 'while [ "$(systemctl is-active fdstore-nopin.service)" != active ]; do sleep 0.5; done'
    assert_eq "$(systemctl show fdstore-pin.service -P NFileDescriptorStore)" 1
    assert_eq "$(systemctl show fdstore-nopin.service -P NFileDescriptorStore)" 0

    systemctl stop fdstore-pin.service fdstore-nopin.service
    assert_eq "$(systemctl show fdstore-pin.service -P NFileDescriptorStore)" 1
    assert_eq "$(systemctl show fdstore-nopin.service -P NFileDescriptorStore)" 0
    assert_eq "$(systemctl show fdstore-pin.service -P SubState)" dead-resources-pinned

    # clean --what=fdstore drops the pinned store, leaving plain dead.
    systemctl clean fdstore-pin.service --what=fdstore
    assert_eq "$(systemctl show fdstore-pin.service -P NFileDescriptorStore)" 0
    assert_eq "$(systemctl show fdstore-pin.service -P SubState)" dead

    touch /testok
    TESTEOF
        chmod +x TEST-80-NOTIFYACCESS.sh
  '';
}
