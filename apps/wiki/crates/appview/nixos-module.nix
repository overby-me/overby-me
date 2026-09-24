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
# The cutover's load runs here too. The service is a DynamicUser with a private
# state directory, so nobody can run `appview import` against it by hand: set
# `import.extraction` (and `import.files`) and `systemctl start
# wiki-appview-import`, which stops the service, loads, asks the cutover's gates
# of what it loaded (`appview verify`), and leaves it to be started again.
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
  appviewEnvironment = {
    PORT = toString cfg.port;
    # StateDirectory is exported by systemd; the Turso file lives under it so
    # it survives restarts.
    APPVIEW_DB = "/var/lib/wiki-appview/appview.db";
    JETSTREAM_URL = cfg.firehoseUrl;
    RUST_LOG = cfg.logFilter;
    APPVIEW_PUBLIC_URL = lib.optionalString (cfg.publicUrl != null) cfg.publicUrl;
    APPVIEW_FRONTEND_ORIGINS = lib.concatStringsSep "," cfg.frontendOrigins;
    APPVIEW_TRUSTED_EMAIL_PDS = lib.concatStringsSep "," cfg.trustedEmailPds;
    APPVIEW_SITE_NAME = cfg.siteName;
    APPVIEW_SITE_OWNER = lib.optionalString (cfg.siteOwner != null) cfg.siteOwner;
    VAPID_PUBLIC_KEY = cfg.vapidPublicKey;
    VAPID_SUBJECT = cfg.vapidSubject;
    APPVIEW_MAIL_FROM = cfg.mailFrom;
    APPVIEW_BOARD_PDS = cfg.board.pds;
    APPVIEW_BOARD_IDENTIFIER = cfg.board.identifier;
    APPVIEW_BOARD_BATCH_SECS = toString cfg.board.batchSeconds;
    APPVIEW_SPACES_PDS = cfg.spaces.pds;
    APPVIEW_SPACES_IDENTIFIER = cfg.spaces.identifier;
    APPVIEW_SPACES_SERVICE = cfg.spaces.service;
    APPVIEW_SPACES_BLOB_LIMIT = toString cfg.spaces.blobLimit;
    APPVIEW_SPACES_ALLOWED_CLIENTS = lib.concatStringsSep "," cfg.spaces.allowedClients;
  };
  # What the service and the cutover's load share: the same state, the same
  # secrets, the same confinement.
  appviewSandbox = {
    # Persistent state for the Turso core+view file. StateDirectory creates and
    # chowns /var/lib/wiki-appview to the DynamicUser.
    StateDirectory = "wiki-appview";
    StateDirectoryMode = "0700";

    EnvironmentFile = lib.filter (file: file != null) [cfg.secretsFile cfg.betterstackTokenFile];

    # Hardening: an unprivileged, sandboxed service with no host access beyond
    # its state dir and the network.
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

    siteName = lib.mkOption {
      type = lib.types.str;
      default = "Wiki";
      description = ''
        What the home is called when the service makes one, which it does the
        first time it starts on a datastore that has none. A wiki loaded with
        `appview import` brings its own home and keeps its own name.
      '';
    };

    siteOwner = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "did:plc:exampleexampleexample00";
      description = ''
        A DID seated as an owner of the home at every start. The home's owners
        run the site: they start what sits at the top of it, and see the reports.
        This is the operator's way in, to a new site that nobody owns yet, or to
        a loaded one none of whose owners can sign in. It is as good as root on
        the wiki's administration, so name an account you hold.
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
        without it no notification is sent), `APPVIEW_SMTP_URL` (see `mailFrom`),
        `APPVIEW_BOARD_PASSWORD` (see `board`), `APPVIEW_SPACES_PASSWORD` (see
        `spaces`) and `BETTERSTACK_SOURCE_TOKEN`.
      '';
    };

    board = {
      pds = lib.mkOption {
        type = lib.types.str;
        default = "";
        example = "https://bsky.social";
        description = ''
          The PDS of the account the ballot boards are published from. With
          `identifier` and `APPVIEW_BOARD_PASSWORD` in `secretsFile` (an app
          password of that account), the board of a poll opened as public is
          published there as records, for anyone to read and to mirror with
          `wiki-board-mirror`. Unset, nothing is published, and each board is
          served by the AppView to whoever may see its counts.
        '';
      };
      identifier = lib.mkOption {
        type = lib.types.str;
        default = "";
        example = "afstemninger.example.org";
        description = ''
          The handle or DID of the board account: one kept for nothing but
          boards, whose keys the organization holds.
        '';
      };
      batchSeconds = lib.mkOption {
        type = lib.types.ints.positive;
        default = 30;
        description = ''
          How often waiting ballots are published. They go out at least three
          together and shuffled, or all at the close, so that no ballot's
          publication says when it was cast.
        '';
      };
    };

    spaces = {
      pds = lib.mkOption {
        type = lib.types.str;
        default = "";
        example = "https://pds.example.org";
        description = ''
          The PDS of the organization's account, for mirroring the wiki into
          atproto spaces (`docs/atproto-spaces-redesign.md`): one space per
          group or event, every page, comment and reaction a record in it, held
          by that account. Needs a PDS that has spaces, which as of 2026-09 is
          an alpha nothing real belongs on. With `identifier`, `service` and
          `APPVIEW_SPACES_PASSWORD` in `secretsFile` (an app password of that
          account). Unset, nothing is mirrored. With only some of the four the
          service refuses to start.
        '';
      };
      identifier = lib.mkOption {
        type = lib.types.str;
        default = "";
        example = "wiki.example.org";
        description = "The handle or DID of the organization's account.";
      };
      service = lib.mkOption {
        type = lib.types.str;
        default = "";
        example = "did:web:appview.example.org#wiki_appview";
        description = ''
          This AppView as the spaces name it, a DID and a fragment: who the PDS
          asks whether a user may read or write a space. For a `did:web` of the
          AppView's own domain, the AppView serves the DID document itself
          (`/.well-known/did.json`, pointing at `publicUrl`).
        '';
      };
      allowedClients = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        example = ["https://tools.example.org/board-mirror/client-metadata.json"];
        description = ''
          The `client_id`s of other applications let into the wiki's spaces:
          a board mirror a member runs, say. Every space admits this AppView
          (as `<publicUrl>/client-metadata.json`) and these, and no other
          application, whoever's session it comes with. Each is a client
          metadata document the PDS can fetch, with the application's public
          key in it.
        '';
      };
      blobLimit = lib.mkOption {
        type = lib.types.ints.positive;
        default = 5 * 1024 * 1024;
        description = ''
          The largest file, in bytes, the organization's PDS takes as a blob
          (its `PDS_BLOB_UPLOAD_LIMIT`, 5 MiB as a PDS ships). A larger file is
          not sent: it stays with the AppView alone, and `appview
          mirror-spaces` counts it among what the PDS will not take.
        '';
      };
    };

    mailFrom = lib.mkOption {
      type = lib.types.str;
      default = "";
      example = "RadikalWiki <wiki@example.org>";
      description = ''
        Who invitations are mailed from. With it and `APPVIEW_SMTP_URL` in
        `secretsFile` (such as `smtps://user:password@mail.example.org:465`), an
        address put on a roster is mailed the link to its seat. With neither,
        nothing is mailed and an owner hands the links out. With only one of
        the two the service refuses to start.
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

    import = {
      extraction = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "/root/cutover/extraction.json";
        description = ''
          Where on this host the `extraction.json` to load is
          (`docs/cutover-runbook.md`). Setting it defines
          `wiki-appview-import.service`, which nothing starts but you. A string
          and not a Nix path, on purpose: a path would copy every member's
          address into the world-readable store.
        '';
      };

      files = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "/root/cutover/files";
        description = ''
          The directory `scripts/dump-interim-files.nu` downloaded the interim's
          files into, filed after the load by the same unit. The files in it
          have to be readable by others (the unit runs as nobody in particular);
          the directories above them need not be.
        '';
      };
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
    systemd.services.wiki-appview-import = lib.mkIf (cfg.import.extraction != null) {
      description = "Load a migrated wiki into the AppView's datastore";
      # Never beside the service: starting this stops it, and it stays stopped
      # until someone has looked at what was loaded.
      conflicts = ["wiki-appview.service"];
      environment = appviewEnvironment;

      serviceConfig =
        appviewSandbox
        // {
          Type = "oneshot";
          # As credentials and a bind mount, because the unit's own user can
          # read neither /root nor anything else the files are likely to be in.
          LoadCredential = ["extraction.json:${cfg.import.extraction}"];
          BindReadOnlyPaths = lib.optional (cfg.import.files != null) "${cfg.import.files}:/run/wiki-appview-import-files";
          # The gates last: a red one fails the unit, which is the no-go.
          ExecStart =
            ["${lib.getExe cfg.package} import %d/extraction.json"]
            ++ lib.optional (cfg.import.files != null)
            "${lib.getExe cfg.package} import-files %d/extraction.json /run/wiki-appview-import-files"
            ++ [
              ("${lib.getExe cfg.package} verify %d/extraction.json"
                + lib.optionalString (cfg.import.files != null) " /run/wiki-appview-import-files")
            ];
        };
    };

    systemd.services.wiki-appview = {
      description = "wiki atproto AppView (stateful)";
      wantedBy = ["multi-user.target"];
      after = ["network-online.target"];
      wants = ["network-online.target"];

      environment = appviewEnvironment;

      serviceConfig =
        appviewSandbox
        // {
          ExecStart = "${lib.getExe cfg.package}";
          # A stateful always-on process: restart on any exit so a crashed
          # firehose or panicked task self-heals (the /healthz signal catches a
          # wedged one).
          Restart = "always";
          RestartSec = 2;
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
