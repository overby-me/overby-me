{
  inputs,
  lib,
  hasSecrets ? true,
  ...
}: {
  imports = lib.optionals hasSecrets [
    {
      age.secrets."resolved-secret.conf" = {
        file = inputs.self.secrets.resolved;
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
