{
  name = "45-TIMEDATE";
  patchScript = ''
    cat > TEST-45-TIMEDATE.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # Full port of upstream TEST-45-TIMEDATE testcase_timesyncd: driving
    # `timedatectl ntp-servers` through networkd's SetLinkNTP and observing the
    # timesync1 LinkNTPServers PropertiesChanged signal, plus the
    # RuntimeNTPServers method/property/signal.
    MATCH="type=signal,sender=org.freedesktop.timesync1,member=PropertiesChanged,path=/org/freedesktop/timesync1"

    # networkd side: same jq expression upstream's assert_networkd_ntp uses.
    assert_networkd_ntp() {
        local interface="''${1:?}"
        local value="''${2-}"
        local expr='[.NTP[] | (select(.Family == 2).Address | join(".")), select(has("Server")).Server] | join(" ")'
        local got=""
        for _ in {0..19}; do
            got="$(networkctl status "$interface" --json=short | jq -r "$expr")"
            [[ "$got" == "$value" ]] && return 0
            sleep .5
        done
        echo "assert_networkd_ntp FAILED for $interface: got [$got] want [$value]" >&2
        networkctl status "$interface" --json=short >&2 || true
        return 1
    }

    start_mon() {
        true > /run/tsmon.txt
        timeout 60 busctl monitor --json=short --match="$MATCH" > /run/tsmon.txt 2>&1 &
        MON=$!
        sleep 1
    }
    stop_mon() { kill "$MON" 2>/dev/null || true; wait "$MON" 2>/dev/null || true; }

    # timesyncd side: parse the captured PropertiesChanged the same way upstream's
    # assert_timesyncd_signal does (.payload.data[1].<prop>.data | join(" ")).
    assert_signal_value() {
        local property="$1" value="$2" got=""
        for _ in {0..19}; do
            got="$(jq -Rr --arg p "$property" 'fromjson? | (.payload.data[1][$p].data // empty) | join(" ")' /run/tsmon.txt 2>/dev/null | grep -v '^$' | tail -1)"
            [[ "$got" == "$value" ]] && return 0
            sleep .5
        done
        echo "assert_signal_value FAILED $property: got [$got] want [$value]" >&2
        cat /run/tsmon.txt >&2 || true
        return 1
    }
    assert_no_signal() {
        local property="$1" hit=""
        sleep 3
        hit="$(jq -Rr --arg p "$property" 'fromjson? | select(.payload.data[1][$p]) | "hit"' /run/tsmon.txt 2>/dev/null | head -1)"
        if [[ "$hit" == "hit" ]]; then
            echo "assert_no_signal FAILED: $property signal present" >&2
            cat /run/tsmon.txt >&2
            return 1
        fi
        return 0
    }

    # Create a dummy interface managed by networkd (matches upstream).
    mkdir -p /run/systemd/network/
    cat >/etc/systemd/network/10-ntp99.netdev <<EOF
    [NetDev]
    Name=ntp99
    Kind=dummy
    EOF
    cat >/etc/systemd/network/10-ntp99.network <<EOF
    [Match]
    Name=ntp99

    [Network]
    Address=10.0.0.1/24
    EOF

    systemctl unmask systemd-timesyncd systemd-networkd 2>/dev/null || true
    systemctl restart systemd-networkd
    systemctl restart systemd-timesyncd
    timeout 30 bash -c 'until busctl list 2>/dev/null | grep -q org.freedesktop.network1; do sleep .3; done'
    timeout 30 bash -c 'until busctl list 2>/dev/null | grep -q org.freedesktop.timesync1; do sleep .3; done'
    timeout 30 bash -c 'until networkctl status ntp99 >/dev/null 2>&1; do sleep .3; done'

    # LinkNTPServers: single IP.
    start_mon
    timedatectl ntp-servers ntp99 10.0.0.1
    assert_networkd_ntp ntp99 10.0.0.1
    assert_signal_value LinkNTPServers 10.0.0.1
    stop_mon

    # Setting the same value must NOT emit a PropertiesChanged.
    start_mon
    timedatectl ntp-servers ntp99 10.0.0.1
    assert_networkd_ntp ntp99 10.0.0.1
    assert_no_signal LinkNTPServers
    stop_mon

    # Multiple IPs.
    start_mon
    timedatectl ntp-servers ntp99 10.0.0.1 192.168.0.99
    assert_networkd_ntp ntp99 "10.0.0.1 192.168.0.99"
    assert_signal_value LinkNTPServers "10.0.0.1 192.168.0.99"
    stop_mon

    # Multiple IPs plus server hostnames.
    start_mon
    timedatectl ntp-servers ntp99 10.0.0.1 192.168.0.99 foo.localhost foo 10.11.12.13
    assert_networkd_ntp ntp99 "10.0.0.1 192.168.0.99 foo.localhost foo 10.11.12.13"
    assert_signal_value LinkNTPServers "10.0.0.1 192.168.0.99 foo.localhost foo 10.11.12.13"
    stop_mon

    # RuntimeNTPServers via the direct D-Bus method.
    start_mon
    busctl call org.freedesktop.timesync1 /org/freedesktop/timesync1 org.freedesktop.timesync1.Manager SetRuntimeNTPServers as 4 "10.0.0.1" foo "192.168.99.1" bar
    servers="$(busctl get-property org.freedesktop.timesync1 /org/freedesktop/timesync1 org.freedesktop.timesync1.Manager RuntimeNTPServers)"
    [[ "$servers" == 'as 4 "10.0.0.1" "foo" "192.168.99.1" "bar"' ]]
    assert_signal_value RuntimeNTPServers "10.0.0.1 foo 192.168.99.1 bar"
    stop_mon

    touch /testok
    RIDEOF
  '';
}
