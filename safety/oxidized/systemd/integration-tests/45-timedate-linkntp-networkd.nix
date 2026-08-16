{
  name = "45-TIMEDATE";
  patchScript = ''
    cat > TEST-45-TIMEDATE.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # The networkd half of upstream testcase_timesyncd (assert_networkd_ntp):
    # `timedatectl ntp-servers` routes to networkd's org.freedesktop.network1
    # Manager.SetLinkNTP, which records a per-link NTP override, writes it into
    # the link state file, and exposes it via `networkctl status <if> --json`.
    # This is the exact jq expression upstream's assert_networkd_ntp uses.
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
        cat "/run/systemd/netif/links/$(cat /sys/class/net/$interface/ifindex)" >&2 || true
        return 1
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

    systemctl unmask systemd-networkd 2>/dev/null || true
    systemctl restart systemd-networkd

    # Wait for networkd to own its D-Bus name and to create the dummy link.
    timeout 30 bash -c 'until busctl list 2>/dev/null | grep -q org.freedesktop.network1; do sleep .3; done'
    timeout 30 bash -c 'until networkctl status ntp99 >/dev/null 2>&1; do sleep .3; done'
    networkctl status ntp99 || true

    # Single IP.
    timedatectl ntp-servers ntp99 10.0.0.1
    assert_networkd_ntp ntp99 10.0.0.1

    # Multiple IPs.
    timedatectl ntp-servers ntp99 10.0.0.1 192.168.0.99
    assert_networkd_ntp ntp99 "10.0.0.1 192.168.0.99"

    # Multiple IPs plus server hostnames (mixed address/name classification).
    timedatectl ntp-servers ntp99 10.0.0.1 192.168.0.99 foo.localhost foo 10.11.12.13
    assert_networkd_ntp ntp99 "10.0.0.1 192.168.0.99 foo.localhost foo 10.11.12.13"

    # Revert clears the override, reverting to the (empty) configured NTP.
    timedatectl revert ntp99
    assert_networkd_ntp ntp99 ""

    touch /testok
    RIDEOF
  '';
}
