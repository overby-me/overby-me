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
# StateDirectory, behind a bundled Ferron reverse proxy that terminates TLS and
# forwards to 127.0.0.1:<port>. The StateDirectory holds the Turso file, the
# uploaded files (`blobs/`) and the key that signs file links
# (`appview.db.secret`): a backup takes all three. `/healthz` reports DB-reachable
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
        (no TLS), the sensible default until the domain/name is chosen. Setting
        it switches Ferron to automatic HTTPS, which also needs :443 reachable
        and a writable ACME cache: verify the state dir when a domain lands.
      '';
    };

    publicUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default =
        if cfg.proxyDomain == null
        then null
        else "https://${cfg.proxyDomain}";
      defaultText = lib.literalExpression ''"https://''${proxyDomain}" when proxyDomain is set, else null'';
      description = ''
        Where a browser reaches the AppView. Setting it selects the production
        OAuth client (whose `client_id` is `<publicUrl>/client-metadata.json`,
        which a member's PDS must be able to fetch) and confines outbound
        requests to public addresses. `null` runs a loopback dev client.
      '';
    };

    frontendOrigins = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      example = ["https://radikal.wiki"];
      description = ''
        Browser origins that may call the API (CORS) and that a login may
        return to. Empty keeps the API same-origin.
      '';
    };

    trustedEmailPds = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      example = ["bsky.social" ".host.bsky.network" "pds.example.org"];
      description = ''
        PDS hosts whose word is taken that an account's email address is
        confirmed, which is what hands a signed-in member the invitations sent
        to that address. A leading dot matches every host under it. Empty keeps
        the built-in default, Bluesky's own hosts. Add a PDS only if you trust
        its operator with your roster: one that lies about an address lets its
        owner take that address's seat.
      '';
    };

    firehoseUrl = lib.mkOption {
      type = lib.types.str;
      default = "wss://jetstream2.us-east.bsky.network/subscribe";
      description = "The Jetstream firehose endpoint the consumer connects to.";
    };

    secretsFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Optional path to a file of `KEY=value` lines, loaded via systemd
        EnvironmentFile so none of it enters the store: `APPVIEW_SECRET` (signs
        file links and seals a running poll's issuer key; without it one is
        made and kept in the state directory), `VAPID_PRIVATE_KEY` (Web Push;
        without it no notification is sent) and `BETTERSTACK_SOURCE_TOKEN`.
      '';
    };

    vapidPublicKey = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = ''
        The public half of the Web Push key pair, base64url. It must be the one
        the frontend has compiled in, which every browser's subscription is
        bound to.
      '';
    };

    vapidSubject = lib.mkOption {
      type = lib.types.str;
      default = "";
      example = "mailto:webmaster@example.org";
      description = ''
        The contact a push service is given (`mailto:` or `https:`). Empty uses
        `publicUrl`.
      '';
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
        APPVIEW_PUBLIC_URL = lib.optionalString (cfg.publicUrl != null) cfg.publicUrl;
        APPVIEW_FRONTEND_ORIGINS = lib.concatStringsSep "," cfg.frontendOrigins;
        APPVIEW_TRUSTED_EMAIL_PDS = lib.concatStringsSep "," cfg.trustedEmailPds;
        VAPID_PUBLIC_KEY = cfg.vapidPublicKey;
        VAPID_SUBJECT = cfg.vapidSubject;
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

        EnvironmentFile = lib.filter (file: file != null) [cfg.secretsFile cfg.betterstackTokenFile];

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
