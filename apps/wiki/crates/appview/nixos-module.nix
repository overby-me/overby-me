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
# StateDirectory for the Turso file, behind a bundled Ferron reverse proxy that
# terminates TLS and forwards to 127.0.0.1:<port>. `/healthz` reports DB-reachable
# (+ firehose-connected) for the proxy/uptime check. Structured JSON logs go to
# stdout -> journald; set BETTERSTACK_SOURCE_TOKEN to also ship them to the
# existing sink.
#
# Ferron (nixpkgs `ferron`, a Rust web server) has no upstream NixOS module, so
# this module defines its systemd unit directly with a generated KDL config.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.wiki-appview;
  # The Ferron host block: a catch-all `:80` (plain HTTP) until a domain is
  # chosen; a domain name switches Ferron to automatic HTTPS (Let's Encrypt).
  ferronHost =
    if cfg.proxyDomain == null
    then ":80"
    else cfg.proxyDomain;
  ferronConfig = pkgs.writeText "ferron.kdl" ''
    globals {
      log "/var/log/wiki-appview-proxy/access.log"
      error_log "/var/log/wiki-appview-proxy/error.log"
    }

    ${ferronHost} {
      proxy "http://127.0.0.1:${toString cfg.port}/"
    }
  '';
in {
  options.services.wiki-appview = {
    enable = lib.mkEnableOption "the wiki atproto AppView";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The wiki-appview package (crates/appview/default.nix).";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 8080;
      description = "TCP port the AppView binds on 0.0.0.0 (proxy to this).";
    };

    reverseProxy = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Run the bundled Ferron reverse proxy in front of the AppView.";
    };

    proxyDomain = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        The domain Ferron serves. `null` keeps a plain-HTTP catch-all on :80
        (no TLS) — the sensible default until the domain/name is chosen; setting
        it switches Ferron to automatic HTTPS (which also needs :443 reachable
        and a writable ACME cache — verify the state dir when a domain lands).
      '';
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
        Optional path to a file holding BETTERSTACK_SOURCE_TOKEN (e.g. a
        materialised secret), loaded via systemd EnvironmentFile so it never
        enters the store.
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
      description = "wiki atproto AppView (stateful)";
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

    # The bundled Ferron reverse proxy: TLS-terminate (once a domain is set) and
    # forward to the local AppView. A host may set `reverseProxy = false` to use
    # its own edge.
    systemd.services.wiki-appview-proxy = lib.mkIf cfg.reverseProxy {
      description = "Ferron reverse proxy for the wiki AppView";
      wantedBy = ["multi-user.target"];
      after = ["network-online.target" "wiki-appview.service"];
      wants = ["network-online.target"];

      serviceConfig = {
        ExecStart = "${lib.getExe pkgs.ferron} -c ${ferronConfig}";
        Restart = "always";
        RestartSec = 2;

        # Access/error logs land here; the ACME cert cache (when a domain is set)
        # wants a writable state dir too.
        LogsDirectory = "wiki-appview-proxy";
        StateDirectory = "wiki-appview-proxy";

        # Bind the privileged :80 (and :443 with TLS) as an unprivileged
        # DynamicUser via the one capability that allows it.
        DynamicUser = true;
        AmbientCapabilities = ["CAP_NET_BIND_SERVICE"];
        CapabilityBoundingSet = ["CAP_NET_BIND_SERVICE"];
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        RestrictAddressFamilies = ["AF_INET" "AF_INET6"];
      };
    };
  };
}
