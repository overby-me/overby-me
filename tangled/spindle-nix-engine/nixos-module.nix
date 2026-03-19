# NixOS module: services.tangled-spindles
#
# Declarative multi-runner deployment for tangled-spindle-nix-engine,
# modeled after services.github-runners.
#
# See PLAN.md Phase 7 for the full specification.
#
# Usage:
#   services.tangled-spindles = {
#     runner1 = {
#       enable = true;
#       hostname = "spindle1.example.com";
#       owner = "did:plc:abc123";
#       tokenFile = "/run/secrets/spindle1-token";
#     };
#   };
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.tangled-spindles;

  # Per-runner submodule options
  runnerOpts = {name, ...}: {
    options = {
      enable = lib.mkEnableOption "tangled-spindle runner '${name}'";

      package = lib.mkOption {
        type = lib.types.package;
        default = pkgs.tangled-spindle-nix-engine or (throw "tangled-spindle-nix-engine package not found in pkgs; add the overlay or pass it explicitly");
        description = "The tangled-spindle-nix-engine package to use.";
      };

      hostname = lib.mkOption {
        type = lib.types.str;
        description = "Public hostname of this spindle instance.";
      };

      owner = lib.mkOption {
        type = lib.types.str;
        description = "DID of the spindle owner (e.g. did:plc:abc123).";
      };

      tokenFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Path to the authentication token file. If null, no token is configured.";
      };

      listenAddr = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1:6555";
        description = "Address the HTTP server binds to.";
      };

      jetstreamEndpoint = lib.mkOption {
        type = lib.types.str;
        default = "wss://jetstream1.us-west.bsky.network/subscribe";
        description = "Jetstream WebSocket endpoint URL.";
      };

      plcUrl = lib.mkOption {
        type = lib.types.str;
        default = "https://plc.directory";
        description = "PLC directory URL for DID resolution.";
      };

      dbPath = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Path to the SQLite database. Defaults to StateDirectory.";
      };

      logDir = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Directory for workflow log files. Defaults to LogsDirectory.";
      };

      dev = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Enable development mode (HTTP instead of HTTPS, localhost remapping).";
      };

      engine = {
        maxJobs = lib.mkOption {
          type = lib.types.int;
          default = 2;
          description = "Maximum concurrent workflow executions.";
        };

        queueSize = lib.mkOption {
          type = lib.types.int;
          default = 100;
          description = "Maximum pending jobs in queue.";
        };

        workflowTimeout = lib.mkOption {
          type = lib.types.str;
          default = "5m";
          description = "Maximum duration for a single workflow execution.";
        };

        nixery = lib.mkOption {
          type = lib.types.str;
          default = "nixery.tangled.sh";
          description = "Nixery URL (for compatibility / fallback).";
        };

        extraNixFlags = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [];
          description = "Extra flags passed to nix build.";
        };
      };

      secrets = {
        provider = lib.mkOption {
          type = lib.types.enum ["sqlite" "openbao"];
          default = "sqlite";
          description = "Secrets storage backend.";
        };

        openbao = {
          proxyAddr = lib.mkOption {
            type = lib.types.str;
            default = "http://127.0.0.1:8200";
            description = "OpenBao proxy address.";
          };

          mount = lib.mkOption {
            type = lib.types.str;
            default = "spindle";
            description = "OpenBao KV v2 mount path.";
          };
        };
      };

      extraEnvironment = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = {};
        description = "Additional environment variables for the service.";
      };

      nixPackage = lib.mkOption {
        type = lib.types.package;
        default = pkgs.nix;
        defaultText = lib.literalExpression "pkgs.nix";
        description = "The Nix package to use for builds.";
      };

      extraPackages = lib.mkOption {
        type = lib.types.listOf lib.types.package;
        default = [];
        description = "Extra packages to include in PATH.";
      };

      resourceLimits = {
        memoryMax = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "20G";
          description = "Hard memory limit (systemd MemoryMax). Processes are killed if exceeded.";
        };

        memoryHigh = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "16G";
          description = "Soft memory limit (systemd MemoryHigh). Throttles allocation when exceeded.";
        };

        cpuQuota = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "200%";
          description = "CPU quota (systemd CPUQuota). 100% = one core.";
        };

        tasksMax = lib.mkOption {
          type = lib.types.nullOr (lib.types.either lib.types.int lib.types.str);
          default = 512;
          description = "Maximum number of tasks (processes/threads) the service can spawn.";
        };
      };

      serviceOverrides = lib.mkOption {
        type = lib.types.attrs;
        default = {};
        description = "Override systemd service options.";
      };

      user = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "User to run the service as. Null uses DynamicUser.";
      };

      group = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Group to run the service as. Null uses DynamicUser.";
      };
    };
  };
in {
  options.services.tangled-spindles = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule runnerOpts);
    default = {};
    description = ''
      Declarative tangled-spindle-nix-engine runner instances.
      Each attribute defines an independent runner with its own systemd service,
      state directory, log directory, database, and RBAC configuration.
    '';
  };

  config = let
    enabledRunners = lib.filterAttrs (_: runner: runner.enable) cfg;

    # Generate a systemd service for a single runner instance
    mkRunnerService = name: runner: let
      stateDir = "tangled-spindle/${name}";
      logsDir = "tangled-spindle/${name}";
      runtimeDir = "tangled-spindle/${name}";

      dbPath =
        if runner.dbPath != null
        then runner.dbPath
        else "/var/lib/${stateDir}/spindle.db";
      logDir =
        if runner.logDir != null
        then runner.logDir
        else "/var/log/${logsDir}";

      basePath = lib.makeBinPath ([
          pkgs.bash
          pkgs.coreutils
          pkgs.git
          pkgs.gnutar
          pkgs.gzip
          runner.nixPackage
        ]
        ++ runner.extraPackages);

      environment =
        {
          SPINDLE_SERVER_HOSTNAME = runner.hostname;
          SPINDLE_SERVER_OWNER = runner.owner;
          SPINDLE_SERVER_LISTEN_ADDR = runner.listenAddr;
          SPINDLE_SERVER_JETSTREAM_ENDPOINT = runner.jetstreamEndpoint;
          SPINDLE_SERVER_PLC_URL = runner.plcUrl;
          SPINDLE_SERVER_DB_PATH = dbPath;
          SPINDLE_SERVER_LOG_DIR = logDir;
          SPINDLE_ENGINE = "nix";
          SPINDLE_ENGINE_MAX_JOBS = toString runner.engine.maxJobs;
          SPINDLE_ENGINE_QUEUE_SIZE = toString runner.engine.queueSize;
          SPINDLE_ENGINE_WORKFLOW_TIMEOUT = runner.engine.workflowTimeout;
          SPINDLE_SERVER_NIXERY_URL = runner.engine.nixery;
          SPINDLE_SERVER_SECRETS_PROVIDER = runner.secrets.provider;
        }
        // lib.optionalAttrs (runner.tokenFile != null) {
          SPINDLE_SERVER_TOKEN_FILE = "/var/lib/${stateDir}/token";
        }
        // lib.optionalAttrs runner.dev {
          SPINDLE_DEV = "1";
        }
        // lib.optionalAttrs (runner.secrets.provider == "openbao") {
          SPINDLE_SERVER_SECRETS_OPENBAO_PROXY_ADDR = runner.secrets.openbao.proxyAddr;
          SPINDLE_SERVER_SECRETS_OPENBAO_MOUNT = runner.secrets.openbao.mount;
        }
        // lib.optionalAttrs (runner.engine.extraNixFlags != []) {
          SPINDLE_ENGINE_EXTRA_NIX_FLAGS = lib.concatStringsSep " " runner.engine.extraNixFlags;
        }
        // runner.extraEnvironment;

      # Script to copy token file into state directory with correct permissions
      tokenScript = lib.optionalString (runner.tokenFile != null) (pkgs.writeShellScript "tangled-spindle-${name}-token" ''
        cp "${runner.tokenFile}" "/var/lib/${stateDir}/token"
        chmod 0644 "/var/lib/${stateDir}/token"
      '');
    in {
      "tangled-spindle-${name}" =
        lib.recursiveUpdate {
          description = "Tangled Spindle CI Runner (${name})";
          after = ["network-online.target" "nix-daemon.service"];
          wants = ["network-online.target" "nix-daemon.service"];
          wantedBy = ["multi-user.target"];

          inherit environment;

          path = [runner.package];

          serviceConfig = {
            ExecStartPre = lib.mkIf (runner.tokenFile != null) ["+${tokenScript}"];
            ExecStart = "${runner.package}/bin/tangled-spindle";

            StateDirectory = stateDir;
            LogsDirectory = logsDir;
            RuntimeDirectory = runtimeDir;

            # User isolation
            DynamicUser = runner.user == null;
            User = lib.mkIf (runner.user != null) runner.user;
            Group = lib.mkIf (runner.group != null) runner.group;

            # Filesystem sandboxing
            ProtectSystem = "strict";
            ProtectHome = true;
            PrivateTmp = true;
            PrivateMounts = true;
            PrivateDevices = true;
            PrivateUsers = true;
            ReadWritePaths = [
              "/var/lib/${stateDir}"
              "/var/log/${logsDir}"
            ];

            # Kernel hardening
            ProtectKernelTunables = true;
            ProtectKernelModules = true;
            ProtectKernelLogs = true;
            ProtectControlGroups = true;
            ProtectClock = true;
            ProtectHostname = true;
            ProtectProc = "invisible";

            # Privilege hardening
            NoNewPrivileges = true;
            RemoveIPC = true;
            RestrictSUIDSGID = true;
            RestrictNamespaces = true;
            RestrictRealtime = true;
            RestrictAddressFamilies = ["AF_INET" "AF_INET6" "AF_UNIX" "AF_NETLINK"];

            # Nix/Node need writable memory for JIT
            MemoryDenyWriteExecute = false;

            # Syscall filtering
            SystemCallFilter = [
              "~@clock"
              "~@cpu-emulation"
              "~@module"
              "~@mount"
              "~@obsolete"
              "~@raw-io"
              "~@reboot"
              "~capset"
              "~setdomainname"
              "~sethostname"
            ];

            # Network — steps need network for git, API calls, etc.
            PrivateNetwork = false;

            # Resource limits — prevent runaway workflows from OOM-killing the system.
            MemoryMax = lib.mkIf (runner.resourceLimits.memoryMax != null) runner.resourceLimits.memoryMax;
            MemoryHigh = lib.mkIf (runner.resourceLimits.memoryHigh != null) runner.resourceLimits.memoryHigh;
            CPUQuota = lib.mkIf (runner.resourceLimits.cpuQuota != null) runner.resourceLimits.cpuQuota;
            TasksMax = lib.mkIf (runner.resourceLimits.tasksMax != null) runner.resourceLimits.tasksMax;

            # Restart policy
            Restart = "on-failure";
            RestartSec = 5;

            # Make token file inaccessible after copying
            InaccessiblePaths = lib.mkIf (runner.tokenFile != null) ["-${runner.tokenFile}"];

            # PATH for nix build and step execution
            Environment = ["PATH=${basePath}"];
          };
        }
        runner.serviceOverrides;
    };
  in
    lib.mkIf (enabledRunners != {}) {
      systemd.services = lib.mkMerge (lib.mapAttrsToList mkRunnerService enabledRunners);

      # Runner users need trusted-user status so CI steps can configure
      # binary caches (e.g. cachix use) via the Nix daemon.
      nix.settings.trusted-users =
        lib.mapAttrsToList (
          name: runner:
            if runner.user != null
            then runner.user
            else "tangled-spindle-${name}"
        )
        enabledRunners;
    };
}
