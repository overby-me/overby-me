{
  inputs,
  src,
  lib,
  ...
}: {
  system = "x86_64-linux";

  specialArgs = {
    inherit src inputs lib;
    stateVersion = "25.05";
    hasSecrets = true;
  };

  modules = with inputs.self.nixosModules; [
    inputs.catppuccin.nixosModules.catppuccin
    inputs.home-manager.nixosModules.home-manager
    inputs.self.hardware.dell-xps-9320
    inputs.self.desktops.cosmic
    inputs.self.desktops.gnome
    inputs.ragenix.nixosModules.default
    age
    core
    programs
    services
    catppuccin
    home-manager
    cloud-hypervisor
    tangled-spindle-nix-engine
    ({pkgs, ...}: {
      age.secrets = {
        spindle-token = {
          file = inputs.self.secrets.spindle-token;
          mode = "600";
        };
        ironclaw-env = {
          file = inputs.self.secrets.ironclaw-env;
          owner = "ironclaw";
          mode = "600";
        };
        searxng-env = {
          file = inputs.self.secrets.searxng-env;
          mode = "600";
        };
        stalwart-admin-password = {
          file = inputs.self.secrets.stalwart-admin-password;
          owner = "stalwart-mail";
          mode = "600";
        };
      };

      services = {
        ironclaw = {
          enable = true;
          logLevel = "ironclaw=info";
          environmentFile = "/run/agenix/ironclaw-env";
          activatedChannels = ["matrix" "mail" "calendar" "contacts"];
          matrix = {
            homeserver = "https://matrix.overby.me";
            dmPolicy = "allowlist";
            allowFrom = ["@niclas:overby.me"];
          };
          mail = {
            jmapUrl = "https://mail.overby.me";
            dmPolicy = "allowlist";
            allowFrom = ["niclas@overby.me"];
            sendFromName = "IronClaw";
          };
          calendar = {
            caldavUrl = "https://mail.overby.me";
            calendarName = "default";
          };
          contacts = {
            carddavUrl = "https://mail.overby.me";
            addressbookName = "default";
          };
          searxng = {
            instanceUrl = "https://search.overby.me";
          };
        };

        stalwart-mail = {
          enable = true;
          settings = {
            server.hostname = "mail.overby.me";

            server.listener = {
              "http" = {
                bind = ["[::]:8443"];
                protocol = "http";
              };
              "smtp" = {
                bind = ["[::]:25"];
                protocol = "smtp";
              };
              "submissions" = {
                bind = ["[::]:465"];
                protocol = "smtp";
                tls.implicit = true;
              };
              "imaps" = {
                bind = ["[::]:993"];
                protocol = "imap";
                tls.implicit = true;
              };
            };

            lookup.default.hostname = "mail.overby.me";
            lookup.default.domain = "agent.overby.me";

            session = {
              auth.mechanisms = "[plain]";
              auth.directory = "'internal'";
              rcpt.directory = "'internal'";
            };

            authentication.fallback-admin = {
              user = "admin";
              secret = "%{file:/run/agenix/stalwart-admin-password}%";
            };
          };
          credentials = {
            stalwart-admin-password = "/run/agenix/stalwart-admin-password";
          };
        };

        tangled-spindles.default = {
          enable = true;
          hostname = "spindle.overby.me";
          owner = "did:plc:eukcx4amfqmhfrnkix7zwm34";
          tokenFile = "/run/agenix/spindle-token";
          nixPackage = pkgs.pkgsUnstable.nixVersions.latest;
          user = "tangled-spindle";
          group = "tangled-spindle";
          engine = {
            maxJobs = 2;
            queueSize = 100;
            workflowTimeout = "12h";
          };
          resourceLimits = {
            memoryHigh = "16G";
            memoryMax = "20G";
            tasksMax = 4096;
          };
        };

        searx = {
          enable = true;
          settings = {
            server = {
              port = 8888;
              bind_address = "127.0.0.1";
              secret_key = "@SEARX_SECRET_KEY@";
            };
            search = {
              safe_search = 0;
              autocomplete = "duckduckgo";
              default_lang = "en";
            };
            engines = lib.singleton {
              name = "bing";
              disabled = true;
            };
          };
          environmentFile = "/run/agenix/searxng-env";
        };

        caddy = {
          enable = true;
          virtualHosts = {
            "spindle.overby.me" = {
              extraConfig = ''
                reverse_proxy localhost:6555
              '';
            };
            "ironclaw.overby.me" = {
              extraConfig = ''
                reverse_proxy localhost:3000
              '';
            };
            "search.overby.me" = {
              extraConfig = ''
                reverse_proxy localhost:8888
              '';
            };
            "mail.overby.me" = {
              extraConfig = ''
                reverse_proxy localhost:8443
              '';
            };
          };
        };
      };

      security.sudo.wheelNeedsPassword = false;
      networking = {
        hostName = "home";
        networkmanager.unmanaged = ["enp0s13f0u3u4"];
        interfaces.enp0s13f0u3u4 = {
          useDHCP = false;
          ipv4.addresses = [
            {
              address = "10.0.0.2";
              prefixLength = 24;
            }
          ];
        };
        defaultGateway = "10.0.0.1";
        nameservers = ["194.242.2.2"];
        firewall.allowedTCPPorts = [22 25 80 443 465 993];
      };
    })
  ];
}
