{
  name = "45-TIMEDATE";
  patchScript = ''
    cat > TEST-45-TIMEDATE.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # The RuntimeNTPServers half of upstream testcase_timesyncd (self-contained,
    # no networkd): the timesync1 Manager exposes a RuntimeNTPServers property and
    # a SetRuntimeNTPServers(as) method that emits PropertiesChanged on change.
    systemctl unmask systemd-timesyncd 2>/dev/null || true
    systemctl start systemd-timesyncd
    timeout 15 bash -c 'until busctl list 2>/dev/null | grep -q org.freedesktop.timesync1; do sleep .3; done'

    timeout 20 busctl monitor --match="type=signal,sender=org.freedesktop.timesync1,member=PropertiesChanged" >/run/tsmon.txt 2>&1 &
    MON=$!
    sleep 1

    busctl call org.freedesktop.timesync1 /org/freedesktop/timesync1 org.freedesktop.timesync1.Manager SetRuntimeNTPServers as 4 "10.0.0.1" foo "192.168.99.1" bar
    servers="$(busctl get-property org.freedesktop.timesync1 /org/freedesktop/timesync1 org.freedesktop.timesync1.Manager RuntimeNTPServers)"
    echo "servers=[$servers]"
    [[ "$servers" == 'as 4 "10.0.0.1" "foo" "192.168.99.1" "bar"' ]]

    sleep 1
    kill "$MON" 2>/dev/null || true
    cat /run/tsmon.txt
    grep -q RuntimeNTPServers /run/tsmon.txt

    # A no-op re-set must NOT emit a PropertiesChanged (change-only behaviour).
    true > /run/tsmon.txt
    timeout 20 busctl monitor --match="type=signal,sender=org.freedesktop.timesync1,member=PropertiesChanged" >/run/tsmon.txt 2>&1 &
    MON2=$!
    sleep 1
    busctl call org.freedesktop.timesync1 /org/freedesktop/timesync1 org.freedesktop.timesync1.Manager SetRuntimeNTPServers as 4 "10.0.0.1" foo "192.168.99.1" bar
    sleep 1
    kill "$MON2" 2>/dev/null || true
    (! grep -q RuntimeNTPServers /run/tsmon.txt)

    touch /testok
    RIDEOF
  '';
}
