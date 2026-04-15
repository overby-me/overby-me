{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.onsuccess-basic\\.sh$";
  };
  # Simple OnSuccess/OnFailure test: verify trigger chains fire correctly.
  patchScript = ''
    cat > TEST-23-UNIT-FILE.onsuccess-basic.sh << 'TESTEOF'
#!/usr/bin/env bash
set -eux
set -o pipefail

. "$(dirname "$0")"/util.sh

at_exit() {
    set +e
    systemctl stop trigger-source.service trigger-target.service 2>/dev/null
    systemctl stop fail-source.service fail-target.service 2>/dev/null
    rm -f /run/systemd/system/trigger-*.service /run/systemd/system/fail-*.service
    rm -f /tmp/onsuccess-fired /tmp/onfailure-fired
    systemctl daemon-reload
}
trap at_exit EXIT

# Test OnSuccess=
cat > /run/systemd/system/trigger-source.service <<EOF
[Unit]
OnSuccess=trigger-target.service
[Service]
Type=simple
ExecStart=/usr/bin/true
EOF

cat > /run/systemd/system/trigger-target.service <<EOF
[Service]
Type=simple
ExecStart=/usr/bin/bash -c 'touch /tmp/onsuccess-fired && sleep infinity'
EOF

systemctl daemon-reload

rm -f /tmp/onsuccess-fired

systemctl start trigger-source.service

timeout 30 bash -c 'while [ ! -f /tmp/onsuccess-fired ]; do sleep 0.5; done'

assert_eq "$(systemctl is-active trigger-target.service)" "active"

systemctl stop trigger-target.service

# Test OnFailure=
cat > /run/systemd/system/fail-source.service <<EOF
[Unit]
OnFailure=fail-target.service
[Service]
Type=simple
ExecStart=/usr/bin/false
EOF

cat > /run/systemd/system/fail-target.service <<EOF
[Service]
Type=simple
ExecStart=/usr/bin/bash -c 'touch /tmp/onfailure-fired && sleep infinity'
EOF

systemctl daemon-reload

rm -f /tmp/onfailure-fired

(! systemctl start fail-source.service) || true

timeout 30 bash -c 'while [ ! -f /tmp/onfailure-fired ]; do sleep 0.5; done'

assert_eq "$(systemctl is-active fail-target.service)" "active"

systemctl stop fail-target.service

touch /testok
TESTEOF
    chmod +x TEST-23-UNIT-FILE.onsuccess-basic.sh
  '';
}
