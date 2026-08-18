{
  pkgs,
  lib,
  ...
}: {
  boot.kernel.sysctl."net.ipv4.ip_forward" = 1;

  networking.firewall.extraCommands = ''
    iptables -t nat -A POSTROUTING -s 192.168.100.0/24 -j MASQUERADE
  '';

  networking.firewall.interfaces.vmtap0 = {
    allowedUDPPorts = [53 67];
    allowedTCPPorts = [53];
  };

  # Create and configure vmtap0 via a oneshot service instead of networkd netdev.
  # systemd-networkd refuses to set TAP ownership for non-system users (UID >= 1000),
  # so we use ip-tuntap directly.
  systemd.services.vmtap0 = {
    description = "TAP device for cloud-hypervisor VMs";
    wantedBy = ["multi-user.target"];
    before = ["dnsmasq-vm.service"];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = pkgs.writeShellScript "vmtap0-up" ''
        ${pkgs.iproute2}/bin/ip tuntap add dev vmtap0 mode tap user overby.me
        ${pkgs.iproute2}/bin/ip addr add 192.168.100.1/24 dev vmtap0
        ${pkgs.iproute2}/bin/ip link set vmtap0 up
      '';
      ExecStop = pkgs.writeShellScript "vmtap0-down" ''
        ${pkgs.iproute2}/bin/ip link del vmtap0
      '';
    };
  };

  # Run a dedicated dnsmasq instance on vmtap0 to serve DHCP + DNS to VMs
  systemd.services.dnsmasq-vm = {
    description = "DHCP/DNS for cloud-hypervisor VMs";
    after = ["vmtap0.service"];
    requires = ["vmtap0.service"];
    wantedBy = ["multi-user.target"];
    serviceConfig = {
      ExecStart = lib.concatStringsSep " " [
        "${pkgs.dnsmasq}/bin/dnsmasq"
        "--keep-in-foreground"
        "--interface=vmtap0"
        "--bind-interfaces"
        "--dhcp-range=192.168.100.100,192.168.100.200,24h"
        "--dhcp-option=option:router,192.168.100.1"
        "--dhcp-option=option:dns-server,1.1.1.1,8.8.8.8"
        "--no-resolv"
        "--log-dhcp"
      ];
      Restart = "on-failure";
    };
  };
}
