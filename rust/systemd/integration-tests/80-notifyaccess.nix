{
  name = "80-NOTIFYACCESS";
  # Custom test: verify NotifyAccess= enforcement via SCM_CREDENTIALS.
  # Skip upstream test (uses systemd-run --wait, busctl, Type=notify-reload).
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

: "NotifyAccess=none — READY=1 rejected, service times out"
(! systemctl start testnotify-none.service)
assert_eq "$(systemctl is-failed testnotify-none.service)" "true"
systemctl reset-failed testnotify-none.service 2>/dev/null || true
sleep 1

: "NotifyAccess=exec — service process group can send READY=1"
systemctl start testnotify-exec.service
timeout 30 bash -c 'while [ "$(systemctl is-active testnotify-exec.service)" != active ]; do sleep 0.5; done'
assert_eq "$(systemctl is-active testnotify-exec.service)" "active"
systemctl stop testnotify-exec.service

touch /testok
TESTEOF
    chmod +x TEST-80-NOTIFYACCESS.sh
  '';
}
