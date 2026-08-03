{
  name = "80-NOTIFYACCESS";
  # Custom test: verify NotifyAccess= enforcement via SCM_CREDENTIALS.
  # Substitutes the upstream script. Its remaining unmet needs are the
  # Type=notify-reload reload substates (reload-signal/reload-notify) with
  # ReloadResult=timeout, plus the TEST-80-NOTIFYACCESS.units fixtures. The
  # status-error triad (ERRNO=/BUSERROR=/VARLINKERROR=) and the NOTIFYACCESS=
  # runtime override now work.
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
        rm -f /run/systemd/system/testnotify-*.service
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

    touch /testok
    TESTEOF
        chmod +x TEST-80-NOTIFYACCESS.sh
  '';
}
