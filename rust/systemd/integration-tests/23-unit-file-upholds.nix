{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.upholds\\.sh$";
  };
  # Custom Upholds test: the upstream test sends SIGUSR1 to TEST-23-UNIT-FILE.service
  # (which in upstream is the test runner itself), but our framework runs tests
  # directly in a shell, not as a systemd service. So we rewrite the test to use
  # file markers instead of signals.
  patchScript = ''
    cat > TEST-23-UNIT-FILE.upholds.sh << 'TESTEOF'
#!/usr/bin/env bash
set -eux
set -o pipefail

. "$(dirname "$0")"/util.sh

at_exit() {
    set +e
    systemctl stop upholds-source.service 2>/dev/null
    systemctl stop upholds-fail.service 2>/dev/null
    systemctl stop upholds-uphold.service 2>/dev/null
    systemctl stop upholds-short-lived.service 2>/dev/null
    systemctl stop upholds-direct-uphold.service 2>/dev/null
    systemctl stop upholds-direct-short.service 2>/dev/null
    rm -f /run/systemd/system/upholds-*.service
    rm -f /tmp/upholds-chain-done /tmp/upholds-counter /tmp/upholds-script.sh
    systemctl daemon-reload
}
trap at_exit EXIT

# Create a counter script as a separate file — inline bash -c one-liners
# can have escaping issues when created via heredocs in unit files.
cat > /tmp/upholds-script.sh << 'SCRIPTEOF'
#!/usr/bin/bash
set -e
counter=0
if [ -f /tmp/upholds-counter ]; then
    counter=$(cat /tmp/upholds-counter)
fi
counter=$((counter + 1))
echo $counter > /tmp/upholds-counter
if [ $counter -ge 5 ]; then
    touch /tmp/upholds-chain-done
fi
exec /usr/bin/sleep 1.5
SCRIPTEOF
chmod +x /tmp/upholds-script.sh

# -- Part 1: Test the OnSuccess -> OnFailure -> Upholds chain --
# Chain: source (exit 0) -> OnSuccess -> fail (exit 1) -> OnFailure -> uphold (sleep inf, Upholds=short-lived)

cat > /run/systemd/system/upholds-source.service <<EOF
[Unit]
Description=Succeeding unit (OnSuccess chain start)
OnSuccess=upholds-fail.service
[Service]
Type=simple
ExecStart=/usr/bin/true
EOF

cat > /run/systemd/system/upholds-fail.service <<EOF
[Unit]
Description=Failing unit (OnFailure chain middle)
OnFailure=upholds-uphold.service
[Service]
Type=simple
ExecStart=/usr/bin/false
EOF

cat > /run/systemd/system/upholds-uphold.service <<EOF
[Unit]
Description=Upholding unit
Upholds=upholds-short-lived.service
[Service]
Type=simple
ExecStart=/usr/bin/sleep infinity
EOF

cat > /run/systemd/system/upholds-short-lived.service <<EOF
[Unit]
Description=Short-lived unit (upheld, restarts via Upholds)
StopWhenUnneeded=yes
StartLimitBurst=15
StartLimitIntervalSec=3600
[Service]
Type=simple
ExecStart=/tmp/upholds-script.sh
EOF

systemctl daemon-reload

rm -f /tmp/upholds-counter /tmp/upholds-chain-done

systemctl start upholds-source.service

# Wait for the chain to fire and short-lived to have been restarted 5 times
timeout 120 bash -c 'while [ ! -f /tmp/upholds-chain-done ]; do sleep 0.5; done'

assert_eq "$(systemctl is-active upholds-uphold.service)" "active"

systemctl stop upholds-uphold.service
systemctl stop upholds-short-lived.service 2>/dev/null || true

# -- Part 2: Test Upholds in isolation (no OnSuccess/OnFailure chain) --

rm -f /tmp/upholds-counter /tmp/upholds-chain-done

cat > /run/systemd/system/upholds-direct-uphold.service <<EOF
[Unit]
Description=Direct upholding unit
Upholds=upholds-direct-short.service
[Service]
Type=simple
ExecStart=/usr/bin/sleep infinity
EOF

cat > /run/systemd/system/upholds-direct-short.service <<EOF
[Unit]
Description=Short-lived service upheld directly
StartLimitBurst=15
StartLimitIntervalSec=3600
[Service]
Type=simple
ExecStart=/tmp/upholds-script.sh
EOF

systemctl daemon-reload

systemctl start upholds-direct-uphold.service

timeout 30 bash -c 'while [ "$(systemctl is-active upholds-direct-short.service 2>/dev/null)" != active ]; do sleep 0.5; done'

# Wait for it to be restarted at least 5 times
timeout 60 bash -c 'while [ ! -f /tmp/upholds-chain-done ]; do sleep 0.5; done'

systemctl stop upholds-direct-uphold.service
systemctl stop upholds-direct-short.service 2>/dev/null || true
rm -f /run/systemd/system/upholds-direct-*.service

touch /testok
TESTEOF
    chmod +x TEST-23-UNIT-FILE.upholds.sh
  '';
}
