# tangled-spindle-nix

A Rust reimplementation of the [Tangled Spindle](https://docs.tangled.org/spindles.html) CI runner that replaces Docker-based isolation with **native Nix** for dependency management and **systemd service-level sandboxing** for isolation.

Includes a NixOS module (`services.tangled-spindles`) for declarative multi-runner deployment, modeled after [`services.github-runners`](https://search.nixos.org/options?channel=unstable&query=services.github-runners).

## Why?

The [upstream spindle](https://tangled.org/tangled.org/core/tree/master/spindle) is written in Go and requires Docker (or Podman) to isolate pipeline steps. Every step runs in a fresh container whose image is built on-the-fly via [Nixery](https://nixery.dev). This works well, but on a NixOS host it introduces unnecessary overhead:

- **Docker is heavy** — a full container runtime, overlay filesystem driver, and root-equivalent daemon
- **Nixery round-trips are redundant** — the Nix store already has (or can build) every package locally
- **systemd already provides isolation** — a hardened systemd service with `DynamicUser=`, `PrivateTmp=`, `ProtectSystem=strict`, seccomp, and cgroups gives container-grade sandboxing without a container runtime. All child processes (workflow steps) inherit this automatically.
- **NixOS modules are the idiomatic deployment method** — declaratively spin up N runners with per-runner configuration, secrets, and hardening

## Architecture

```text
                    ┌──────────────────────────────────┐
                    │       tangled-spindle-nix         │
                    │                                   │
 Jetstream ───────► │  Ingester (member/repo/collab)    │
 (AT Protocol)      │           │                       │
                    │           ▼                        │
 Knot events ─────► │  Event Consumer (sh.tangled.pipe) │
                    │           │                        │
                    │           ▼                        │
                    │       Job Queue                    │
                    │           │                        │
                    │           ▼                        │
                    │   ┌───────────────────┐            │
                    │   │  Engine: nix      │            │
                    │   │                   │            │
                    │   │ 1. nix build      │            │
                    │   │ 2. fork/exec step │            │
                    │   │ 3. stream logs    │            │
                    │   └───────────────────┘            │
                    │                                    │
                    │   HTTP: /events /logs /xrpc/*      │
                    └──────────────────────────────────┘
```

Instead of Docker containers, each workflow step is executed as a **child process** of the runner daemon. The runner's systemd service provides all sandboxing — child processes inherit it automatically, just like `github-runners`:

| Concern | Docker (upstream) | nix engine (this project) |
|---------|-------------------|---------------------------|
| Process isolation | Container namespaces | systemd service-level sandboxing (inherited by children) |
| Dependencies | Nixery image pull over HTTPS | `nix build` from local store |
| Filesystem | Overlay FS | Service-level `ProtectSystem=strict`, `PrivateTmp=`, `ReadWritePaths=` |
| User isolation | Container root mapped to host uid | `DynamicUser=true` on the runner service |
| Resource limits | Docker cgroup limits | Service-level `CPUQuota=`, `MemoryMax=`, `TasksMax=` |
| Network isolation | Docker bridge | Service-level `PrivateNetwork=` (configurable) |
| Log streaming | Docker attach stdout/stderr | Piped stdout/stderr from child process |
| Cleanup | Container removal | Workspace directory cleanup |

## NixOS Module

```nix
{
  services.tangled-spindles = {
    runner1 = {
      enable = true;
      hostname = "spindle1.example.com";
      owner = "did:plc:abc123";
      tokenFile = "/run/secrets/spindle1-token";
    };

    runner2 = {
      enable = true;
      hostname = "spindle2.example.com";
      owner = "did:plc:def456";
      tokenFile = "/run/secrets/spindle2-token";
      engine.maxJobs = 4;
      engine.workflowTimeout = "30m";
      secrets.provider = "openbao";
    };
  };
}
```

Each entry produces an independent `tangled-spindle-{name}.service` systemd unit with its own state directory, log directory, database, and RBAC configuration. Sandboxing is applied at the service level and inherited by all workflow step child processes — no `systemd-run`, polkit rules, or elevated capabilities needed.

## Compatibility

This project is **wire-compatible** with the upstream Go spindle:

- Same WebSocket event format on `/events` and `/logs/{knot}/{rkey}/{name}`
- Same XRPC endpoints and service auth
- Same JSON log line schema
- Same `SPINDLE_SERVER_*` environment variable configuration
- Runs the same `.tangled/workflows/*.yml` pipeline manifests

An appview or tangled.org frontend that works with the Go spindle works identically with this one.

## Development

```bash
# Enter the dev shell
cd tangled-spindle-nix
just build   # cargo build
just test    # cargo test
just check   # cargo clippy + cargo fmt --check
just run     # cargo run (requires SPINDLE_SERVER_* env vars)
```

## Status

🚧 **Early development** — see [PLAN.md](PLAN.md) for the full implementation plan and phase breakdown.

## License

MIT