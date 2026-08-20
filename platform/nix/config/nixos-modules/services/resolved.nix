{
  lib,
  hasSecrets ? true,
  ...
}: {
  imports = lib.optionals hasSecrets [
    {
      secretspec.secrets.RESOLVED_SECRET_CONF = {
        encoding = "base64";
        path = "/etc/systemd/resolved.conf.d/9-secret.conf";
        owner = "systemd-resolve";
        group = "systemd-resolve";
        mode = "600";
      };
    }
  ];

  services.resolved = {
    enable = true;
    settings.Resolve = {
      DNSOverTLS = true;
      MulticastDNS = "resolve";
    };
  };
}
