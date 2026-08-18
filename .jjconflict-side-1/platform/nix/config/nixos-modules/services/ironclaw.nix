# IronClaw, a secure personal AI assistant: PostgreSQL with pgvector, the
# service, and an environment file for its secrets.
#
# After the first deployment, run `ironclaw onboard` on the device to finish
# NEAR AI authentication and secrets setup, or pre-populate that environment
# file yourself.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.ironclaw;
  # URL-encode the host so a Unix socket path becomes a valid URL component,
  # and double the % signs so systemd does not read them as specifiers.
  urlEncodeHost = host:
    lib.replaceStrings ["/"] ["%%2F"] host;

  # v0.18.0 loads channels from WASM_CHANNELS_DIR as a flat directory:
  # <name>.wasm plus <name>.capabilities.json per channel.
  matrixConfigJson = lib.toJSON {
    inherit (cfg.matrix) homeserver;
    dm_policy = cfg.matrix.dmPolicy;
    allow_from = cfg.matrix.allowFrom;
    room_ids = cfg.matrix.roomIds;
    require_mention = cfg.matrix.requireMention;
  };

  blueskyConfigJson = lib.toJSON {
    pds_url = cfg.bluesky.pdsUrl;
    dm_policy = cfg.bluesky.dmPolicy;
    allow_from = cfg.bluesky.allowFrom;
    respond_to_mentions = cfg.bluesky.respondToMentions;
  };

  signalConfigJson = lib.toJSON {
    api_url = cfg.signal.apiUrl;
    dm_policy = cfg.signal.dmPolicy;
    allow_from = cfg.signal.allowFrom;
    group_ids = cfg.signal.groupIds;
    require_mention = cfg.signal.requireMention;
  };

  mailConfigJson = lib.toJSON {
    jmap_url = cfg.mail.jmapUrl;
    dm_policy = cfg.mail.dmPolicy;
    allow_from = cfg.mail.allowFrom;
    mailbox_name = cfg.mail.mailboxName;
    send_from_name = cfg.mail.sendFromName;
  };

  calendarConfigJson = lib.toJSON {
    caldav_url = cfg.calendar.caldavUrl;
    calendar_name = cfg.calendar.calendarName;
    poll_interval_ms = cfg.calendar.pollIntervalMs;
  };

  contactsConfigJson = lib.toJSON {
    carddav_url = cfg.contacts.carddavUrl;
    addressbook_name = cfg.contacts.addressbookName;
  };

  searxngConfigJson = lib.toJSON {
    instance_url = cfg.searxng.instanceUrl;
  };

  # Extract hostname from a URL like "https://matrix.overby.me" -> "matrix.overby.me"
  extractHost = url:
    lib.head (lib.match "https?://([^/:]+).*" url);

  matrixAllowlistJson = lib.toJSON [
    {
      host = extractHost cfg.matrix.homeserver;
      path_prefix = "/_matrix/";
    }
  ];

  blueskyAllowlistJson = lib.toJSON [
    {
      host = extractHost cfg.bluesky.pdsUrl;
      path_prefix = "/xrpc/";
    }
  ];

  signalApiHost = extractHost cfg.signal.apiUrl;
  signalAllowlistJson = lib.toJSON [
    {
      host = signalApiHost;
      path_prefix = "/";
    }
  ];

  mailJmapHost = extractHost cfg.mail.jmapUrl;
  mailAllowlistJson = lib.toJSON [
    {
      host = mailJmapHost;
      path_prefix = "/";
    }
  ];

  calendarHost = extractHost cfg.calendar.caldavUrl;
  calendarAllowlistJson = lib.toJSON [
    {
      host = calendarHost;
      path_prefix = "/";
    }
  ];

  contactsHost = extractHost cfg.contacts.carddavUrl;
  contactsAllowlistJson = lib.toJSON [
    {
      host = contactsHost;
      path_prefix = "/";
    }
  ];

  searxngHost = extractHost cfg.searxng.instanceUrl;
  searxngAllowlistJson = lib.toJSON [
    {
      host = searxngHost;
      path_prefix = "/search";
      methods = ["GET"];
    }
  ];

  wasmChannelsDir =
    pkgs.runCommand "ironclaw-wasm-channels" {
      nativeBuildInputs = [pkgs.jq];
    } ''
      mkdir -p $out

      # Matrix channel
      cp ${pkgs.ironclaw-matrix-channel}/matrix.wasm $out/matrix.wasm
      jq --argjson cfg '${matrixConfigJson}' \
         --argjson allowlist '${matrixAllowlistJson}' \
         '.config = $cfg | .capabilities.http.allowlist = $allowlist' \
        ${pkgs.ironclaw-matrix-channel}/matrix.capabilities.json \
        > $out/matrix.capabilities.json

      # Bluesky channel
      cp ${pkgs.ironclaw-bluesky-channel}/bluesky.wasm $out/bluesky.wasm
      jq --argjson cfg '${blueskyConfigJson}' \
         --argjson allowlist '${blueskyAllowlistJson}' \
         '.config = $cfg | .capabilities.http.allowlist = $allowlist' \
        ${pkgs.ironclaw-bluesky-channel}/bluesky.capabilities.json \
        > $out/bluesky.capabilities.json

      # Telegram channel (built from upstream ironclaw source)
      cp ${cfg.package.telegramChannelWasm}/telegram.wasm $out/telegram.wasm
      cp ${cfg.package.telegramChannelWasm}/telegram.capabilities.json \
         $out/telegram.capabilities.json

      # Signal channel
      cp ${pkgs.ironclaw-signal-channel}/signal.wasm $out/signal.wasm
      jq --argjson cfg '${signalConfigJson}' \
         --argjson allowlist '${signalAllowlistJson}' \
         '.config = $cfg | .capabilities.http.allowlist = $allowlist' \
        ${pkgs.ironclaw-signal-channel}/signal.capabilities.json \
        > $out/signal.capabilities.json

      # Mail channel
      cp ${pkgs.ironclaw-mail-channel}/mail.wasm $out/mail.wasm
      jq --argjson cfg '${mailConfigJson}' \
         --argjson allowlist '${mailAllowlistJson}' \
         '.config = $cfg | .capabilities.http.allowlist = $allowlist' \
        ${pkgs.ironclaw-mail-channel}/mail.capabilities.json \
        > $out/mail.capabilities.json

      # Calendar channel
      cp ${pkgs.ironclaw-calendar-channel}/calendar.wasm $out/calendar.wasm
      jq --argjson cfg '${calendarConfigJson}' \
         --argjson allowlist '${calendarAllowlistJson}' \
         '.config = $cfg | .capabilities.http.allowlist = $allowlist' \
        ${pkgs.ironclaw-calendar-channel}/calendar.capabilities.json \
        > $out/calendar.capabilities.json

      # Contacts channel
      cp ${pkgs.ironclaw-contacts-channel}/contacts.wasm $out/contacts.wasm
      jq --argjson cfg '${contactsConfigJson}' \
         --argjson allowlist '${contactsAllowlistJson}' \
         '.config = $cfg | .capabilities.http.allowlist = $allowlist' \
        ${pkgs.ironclaw-contacts-channel}/contacts.capabilities.json \
        > $out/contacts.capabilities.json

    '';

  wasmToolsDir =
    pkgs.runCommand "ironclaw-wasm-tools" {
      nativeBuildInputs = [pkgs.jq];
    } ''
      mkdir -p $out

      # SearXNG tool
      cp ${pkgs.ironclaw-searxng-tool}/searxng.wasm $out/searxng.wasm
      jq --argjson cfg '${searxngConfigJson}' \
         --argjson allowlist '${searxngAllowlistJson}' \
         '.config = $cfg | .capabilities.http.allowlist = $allowlist' \
        ${pkgs.ironclaw-searxng-tool}/searxng-tool.capabilities.json \
        > $out/searxng.capabilities.json

      # Pixtral image generation tool
      cp ${pkgs.ironclaw-pixtral-tool}/pixtral.wasm $out/pixtral.wasm
      cp ${pkgs.ironclaw-pixtral-tool}/pixtral-tool.capabilities.json \
         $out/pixtral.capabilities.json
    '';

  # List of channel names to auto-activate on startup.
  activatedChannelsJson = lib.toJSON cfg.activatedChannels;
in {
  options.services.ironclaw = {
    enable = lib.mkEnableOption "IronClaw AI assistant";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.ironclaw;
      defaultText = lib.literalExpression "pkgs.ironclaw";
      description = "The IronClaw package to use.";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "ironclaw";
      description = "System user under which IronClaw runs.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "ironclaw";
      description = "System group under which IronClaw runs.";
    };

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/ironclaw";
      description = "Directory for IronClaw persistent state.";
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Path to an environment file loaded by the systemd unit.
        Use this to supply secrets such as API keys without putting
        them in the Nix store.  Expected variables:

          LLM_BACKEND=nearai
          NEARAI_API_KEY=<your-key>
          SECRETS_MASTER_KEY=<hex-encoded-32-byte-key>
          HTTP_WEBHOOK_SECRET=<random-secret>

        Channel-specific tokens (e.g. MATRIX_ACCESS_TOKEN) are
        auto-persisted to the encrypted secrets store on startup.

        When using agenix, point this at the decrypted secret path.
      '';
    };

    database = {
      name = lib.mkOption {
        type = lib.types.str;
        default = "ironclaw";
        description = "PostgreSQL database name.";
      };

      host = lib.mkOption {
        type = lib.types.str;
        default = "/run/postgresql";
        description = "PostgreSQL host (use a directory path for Unix socket).";
      };

      createLocally = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether to create the database and enable PostgreSQL locally.";
      };
    };

    logLevel = lib.mkOption {
      type = lib.types.str;
      default = "ironclaw=info";
      description = "RUST_LOG value for the IronClaw service.";
    };

    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Extra command-line arguments passed to the ironclaw binary.";
    };

    http = {
      host = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1";
        description = "HTTP host for the WASM channel runtime webhook server.";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 8080;
        description = "HTTP port for the WASM channel runtime webhook server.";
      };
    };

    activatedChannels = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      example = ["matrix" "bluesky"];
      description = "Channel names to auto-activate on startup.";
    };

    matrix = {
      homeserver = lib.mkOption {
        type = lib.types.str;
        default = "https://matrix.org";
        description = "Matrix homeserver base URL.";
      };

      dmPolicy = lib.mkOption {
        type = lib.types.enum ["pairing" "allowlist" "open"];
        default = "pairing";
        description = ''
          DM access control policy.
          - pairing: require mutual pairing approval
          - allowlist: only allow users in allowFrom
          - open: accept DMs from anyone
        '';
      };

      allowFrom = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Matrix user IDs allowed to message the bot (used with allowlist/pairing policies).";
      };

      roomIds = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Matrix room IDs to join and monitor. Empty means DM-only.";
      };

      requireMention = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Whether the bot requires an @-mention in rooms to respond.";
      };
    };

    signal = {
      apiUrl = lib.mkOption {
        type = lib.types.str;
        default = "http://localhost:8080";
        description = "signal-cli REST API base URL.";
      };

      dmPolicy = lib.mkOption {
        type = lib.types.enum ["pairing" "allowlist" "open"];
        default = "pairing";
        description = ''
          DM access control policy.
          - pairing: require mutual pairing approval
          - allowlist: only allow numbers in allowFrom
          - open: accept DMs from anyone
        '';
      };

      allowFrom = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Phone numbers allowed to message the bot (used with allowlist/pairing policies).";
      };

      groupIds = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Signal group IDs to monitor. Empty means DM-only.";
      };

      requireMention = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Whether the bot requires a mention in groups to respond.";
      };
    };

    mail = {
      jmapUrl = lib.mkOption {
        type = lib.types.str;
        default = "https://api.fastmail.com";
        description = "JMAP server URL.";
      };

      dmPolicy = lib.mkOption {
        type = lib.types.enum ["allowlist" "open"];
        default = "allowlist";
        description = ''
          Email access control policy.
          - allowlist: only process emails from addresses in allowFrom
          - open: process emails from anyone
        '';
      };

      allowFrom = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Email addresses allowed to message the bot.";
      };

      mailboxName = lib.mkOption {
        type = lib.types.str;
        default = "Inbox";
        description = "JMAP mailbox name to monitor for incoming emails.";
      };

      sendFromName = lib.mkOption {
        type = lib.types.str;
        default = "IronClaw";
        description = "Display name for outgoing email replies.";
      };
    };

    calendar = {
      caldavUrl = lib.mkOption {
        type = lib.types.str;
        default = "http://localhost:8080";
        description = "CalDAV server URL.";
      };

      calendarName = lib.mkOption {
        type = lib.types.str;
        default = "default";
        description = "Calendar name to monitor.";
      };

      pollIntervalMs = lib.mkOption {
        type = lib.types.int;
        default = 60000;
        description = "Polling interval in milliseconds for calendar changes.";
      };
    };

    contacts = {
      carddavUrl = lib.mkOption {
        type = lib.types.str;
        default = "http://localhost:8080";
        description = "CardDAV server URL.";
      };

      addressbookName = lib.mkOption {
        type = lib.types.str;
        default = "default";
        description = "Address book name to monitor.";
      };
    };

    searxng = {
      instanceUrl = lib.mkOption {
        type = lib.types.str;
        default = "http://localhost:8888";
        description = "SearXNG instance URL for web search tool.";
      };
    };

    bluesky = {
      pdsUrl = lib.mkOption {
        type = lib.types.str;
        default = "https://bsky.social";
        description = "AT Protocol PDS URL.";
      };

      dmPolicy = lib.mkOption {
        type = lib.types.enum ["pairing" "allowlist" "open"];
        default = "pairing";
        description = ''
          DM access control policy.
          - pairing: require mutual pairing approval
          - allowlist: only allow DIDs in allowFrom
          - open: accept DMs from anyone
        '';
      };

      allowFrom = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "DIDs or handles allowed to message the bot (used with allowlist/pairing policies).";
      };

      respondToMentions = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether the bot responds to @-mentions on Bluesky posts.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    # ── PostgreSQL with pgvector ─────────────────────────────────────────
    services.postgresql = lib.mkIf cfg.database.createLocally {
      enable = true;
      extensions = ps: [ps.pgvector];
      ensureDatabases = [cfg.database.name];
      ensureUsers = [
        {
          name = cfg.user;
          ensureDBOwnership = true;
        }
      ];
      # pgvector must be created inside the target database; the
      # `ensureExtensions` mechanism is not yet upstream, so we use
      # an initialScript instead.
      settings = {
        shared_preload_libraries = "vector";
      };
    };

    # ── System user / group ──────────────────────────────────────────────
    users.users.${cfg.user} = {
      isSystemUser = true;
      inherit (cfg) group;
      home = cfg.dataDir;
      createHome = true;
      description = "IronClaw service user";
    };

    users.groups.${cfg.group} = {};

    # ── Systemd services ────────────────────────────────────────────────
    systemd.services = {
      # Create the pgvector extension in the ironclaw database after PG starts.
      ironclaw-db-setup = lib.mkIf cfg.database.createLocally {
        description = "IronClaw database schema bootstrap (pgvector)";
        after = ["postgresql.service"];
        requires = ["postgresql.service"];
        wantedBy = ["ironclaw.service"];
        before = ["ironclaw.service"];
        serviceConfig = {
          Type = "oneshot";
          User = "postgres";
          Group = "postgres";
          RemainAfterExit = true;
        };
        script = ''
          # Wait for ensureDatabases (runs in postgresql postStart) to create the DB
          while ! ${config.services.postgresql.package}/bin/psql \
            -d ${lib.escapeShellArg cfg.database.name} -c "SELECT 1" &>/dev/null; do
            sleep 1
          done
          ${config.services.postgresql.package}/bin/psql \
            -d ${lib.escapeShellArg cfg.database.name} \
            -c "CREATE EXTENSION IF NOT EXISTS vector;"
        '';
      };

      # Pre-set activated_channels in the database so WASM channels auto-activate.
      ironclaw-channels-setup = lib.mkIf (cfg.database.createLocally && cfg.activatedChannels != []) {
        description = "IronClaw activated channels bootstrap";
        after = ["postgresql.service" "ironclaw-db-setup.service"];
        requires = ["postgresql.service" "ironclaw-db-setup.service"];
        wantedBy = ["ironclaw.service"];
        before = ["ironclaw.service"];
        serviceConfig = {
          Type = "oneshot";
          User = "postgres";
          Group = "postgres";
          RemainAfterExit = true;
        };
        script = let
          psql = "${config.services.postgresql.package}/bin/psql";
          db = lib.escapeShellArg cfg.database.name;
        in ''
          ${psql} -d ${db} <<'EOSQL'
          INSERT INTO settings (user_id, key, value)
          VALUES ('default', 'activated_channels', '${activatedChannelsJson}'::jsonb)
          ON CONFLICT (user_id, key) DO UPDATE SET value = EXCLUDED.value;
          EOSQL
        '';
      };

      # Seed WASM tool secrets from environment variables declaratively.
      # Each tool with an auth.env_var in its capabilities.json gets its
      # secret auto-imported from the environment file.
      ironclaw-tool-secrets-setup = lib.mkIf (cfg.database.createLocally && cfg.environmentFile != null) {
        description = "IronClaw WASM tool secrets bootstrap";
        after = ["postgresql.service" "ironclaw-db-setup.service"];
        requires = ["postgresql.service" "ironclaw-db-setup.service"];
        wantedBy = ["ironclaw.service"];
        before = ["ironclaw.service"];
        environment = {
          DATABASE_URL = "postgres://${cfg.user}@${urlEncodeHost cfg.database.host}/${cfg.database.name}";
          DATABASE_SSLMODE = "disable";
          IRONCLAW_HOME = cfg.dataDir;
          WASM_TOOLS_DIR = "${wasmToolsDir}";
          ONBOARD_COMPLETED = "true";
        };
        serviceConfig = {
          Type = "oneshot";
          User = cfg.user;
          Group = cfg.group;
          WorkingDirectory = cfg.dataDir;
          EnvironmentFile = cfg.environmentFile;
          RemainAfterExit = true;
        };
        script = ''
          # Ensure local tools dir mirrors the Nix-built WASM_TOOLS_DIR so
          # `ironclaw tool setup` can find capabilities files.
          mkdir -p "${cfg.dataDir}/.ironclaw/tools"
          for f in ${wasmToolsDir}/*; do
            ln -sf "$f" "${cfg.dataDir}/.ironclaw/tools/$(basename "$f")"
          done

          ${cfg.package}/bin/ironclaw tool setup pixtral --no-onboard || true
        '';
      };

      ironclaw = {
        description = "IronClaw AI Assistant";
        documentation = ["https://github.com/nearai/ironclaw"];

        after =
          ["network-online.target"]
          ++ lib.optionals cfg.database.createLocally [
            "postgresql.service"
            "ironclaw-db-setup.service"
          ];
        requires = lib.optionals cfg.database.createLocally [
          "postgresql.service"
          "ironclaw-db-setup.service"
        ];
        wants = ["network-online.target"];
        wantedBy = ["multi-user.target"];

        environment = {
          RUST_LOG = cfg.logLevel;
          DATABASE_URL = "postgres://${cfg.user}@${urlEncodeHost cfg.database.host}/${cfg.database.name}";
          DATABASE_SSLMODE = "disable";
          IRONCLAW_HOME = cfg.dataDir;
          WASM_CHANNELS_DIR = "${wasmChannelsDir}";
          WASM_TOOLS_DIR = "${wasmToolsDir}";
          ONBOARD_COMPLETED = "true";
          # HTTP channel enables the WASM channel runtime
          HTTP_HOST = cfg.http.host;
          HTTP_PORT = toString cfg.http.port;
          # HTTP_WEBHOOK_SECRET and SECRETS_MASTER_KEY must be in environmentFile
        };

        serviceConfig = {
          Type = "simple";
          User = cfg.user;
          Group = cfg.group;
          WorkingDirectory = cfg.dataDir;
          StateDirectory = "ironclaw";
          ExecStart = let
            args = lib.concatStringsSep " " (["--no-onboard"] ++ cfg.extraArgs);
          in "${cfg.package}/bin/ironclaw ${args}";

          Restart = "on-failure";
          RestartSec = 10;

          # Load secrets from the environment file
          EnvironmentFile = lib.mkIf (cfg.environmentFile != null) cfg.environmentFile;

          # Hardening
          NoNewPrivileges = true;
          ProtectSystem = "strict";
          ProtectHome = true;
          PrivateTmp = true;
          ReadWritePaths = [cfg.dataDir];
          ProtectKernelTunables = true;
          ProtectKernelModules = true;
          ProtectControlGroups = true;
          RestrictSUIDSGID = true;
          MemoryDenyWriteExecute = false; # WASM JIT needs W^X
        };
      };
    };

    # Make the CLI available system-wide for manual `ironclaw onboard`, etc.
    environment.systemPackages = [cfg.package];
  };
}
