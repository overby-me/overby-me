# The NixOS systemd unit for the stateful AppView (rewrite kickoff item 11). A
# ready-to-import module: a host wires it in with
#
#   imports = [ ./crates/appview/nixos-module.nix ];
#   services.wiki-appview = {
#     enable = true;
#     package = pkgs.wiki-appview;      # from crates/appview/default.nix
#     port = 8080;
#     firehoseUrl = "wss://jetstream2.us-east.bsky.network/subscribe";
#   };
#
# It runs the binary as a hardened, auto-restarting service with a persistent
# StateDirectory for the Turso file, and expects a Caddy (or other) reverse proxy
# in front terminating TLS and forwarding to 127.0.0.1:<port>. `/healthz` reports
# DB-reachable (+ firehose-configured) for the proxy/uptime check. Structured
# JSON logs go to stdout -> journald; set BETTERSTACK_SOURCE_TOKEN to also ship
# them to the existing sink.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.wiki-appview;
in {
  options.services.wiki-appview = {
    enable = lib.mkEnableOption "the RadikalWiki atproto AppView";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The wiki-appview package (crates/appview/default.nix).";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 8080;
      description = "TCP port the AppView binds on 0.0.0.0 (proxy to this).";
    };

    firehoseUrl = lib.mkOption {
      type = lib.types.str;
      default = "wss://jetstream2.us-east.bsky.network/subscribe";
      description = "The Jetstream firehose endpoint the consumer connects to.";
    };

    betterstackTokenFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Optional path to a file holding BETTERSTACK_SOURCE_TOKEN (e.g. an agenix
        secret), loaded via systemd EnvironmentFile so it never enters the store.
      '';
    };

    logFilter = lib.mkOption {
      type = lib.types.str;
      default = "info";
      description = "RUST_LOG / tracing EnvFilter directive.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.wiki-appview = {
      description = "RadikalWiki atproto AppView (stateful)";
      wantedBy = ["multi-user.target"];
      after = ["network-online.target"];
      wants = ["network-online.target"];

      environment = {
        PORT = toString cfg.port;
        # StateDirectory is exported by systemd; the Turso file lives under it so
        # it survives restarts.
        APPVIEW_DB = "/var/lib/wiki-appview/appview.db";
        JETSTREAM_URL = cfg.firehoseUrl;
        RUST_LOG = cfg.logFilter;
      };

      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package}";
        # A stateful always-on process: restart on any exit so a crashed firehose
        # or panicked task self-heals (the /healthz signal catches a wedged one).
        Restart = "always";
        RestartSec = 2;

        # Persistent state for the Turso core+view file. StateDirectory creates
        # and chowns /var/lib/wiki-appview to the DynamicUser.
        StateDirectory = "wiki-appview";
        StateDirectoryMode = "0700";

        EnvironmentFile = lib.mkIf (cfg.betterstackTokenFile != null) [cfg.betterstackTokenFile];

        # Hardening: an unprivileged, sandboxed service with no host access
        # beyond its state dir and the network.
        DynamicUser = true;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = ["AF_INET" "AF_INET6"];
      };
    };

    # Reverse proxy: TLS-terminate and forward to the local AppView. Kept as a
    # documented default; a host may substitute its own edge.
    services.caddy = lib.mkDefault {
      enable = true;
    };
  };
}
