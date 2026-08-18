# nu-plugin-tramp — PLAN

> A TRAMP-inspired remote filesystem plugin for [Nushell](https://www.nushell.sh/), written in Rust.

---

## 1. Overview

`nu-plugin-tramp` allows Nushell users to transparently access remote files using
TRAMP-style URI paths, e.g.:

```nushell
open /ssh:myvm:/etc/config
ls   /ssh:myvm:/var/log
open /ssh:myvm:/app/config.toml | upsert port 8080 | save /ssh:myvm:/app/config.toml
```

The key design principle — inherited from Emacs TRAMP — is that **the shell
and all its tools stay local**. Only file I/O crosses the transport boundary.
Nushell's structured data model makes this a natural fit: `open` returns a
typed Nushell value; pipes operate locally; `save` writes back remotely.

---

## 1.1 Current Status

**All roadmap phases (1–6) are complete.** The project is feature-complete
for its v1 goals:

| Phase | Name | Status |
|-------|------|--------|
| 1 | MVP | ✅ Complete |
| 2 | Daily Driver | ✅ Complete |
| 3 | Power Features | ✅ Complete |
| 4 | Polish & Usability | ✅ Complete |
| 4.1 | Tab Completion | ✅ Complete |
| 5 | RPC Agent | ✅ Complete |
| 6 | Future / Advanced | ✅ Complete |

Key metrics:

- **295 tests** (175 plugin + 120 agent) — unit, integration, and PTY tests
- **All pre-commit hooks pass** — clippy, rustfmt, statix, markdown lint
- **Two crates** — `nu-plugin-tramp` (plugin) and `tramp-agent` (agent)
- **4 backends** — SSH, Docker, Kubernetes, Sudo (all with RPC agent support)
- **4 cross-compilation targets** — x86_64/aarch64 × Linux/macOS
- **Interactive remote shell** — `tramp shell` command with full PTY support

---

## 2. Repository Layout (Workspace)

```text
nu-plugin-tramp/
├── Cargo.toml              # workspace manifest
├── Cargo.lock
├── PLAN.md                 # this document
├── README.md               # user-facing documentation
├── cross.nix               # Cross-compilation infrastructure for agent binaries
├── default.nix             # Nix package derivations (plugin + agent)
├── hm-module.nix           # Home Manager module
└── crates/
    ├── plugin/             # nu-plugin-tramp — Nushell plugin
    │   ├── Cargo.toml
    │   └── src/
    │       ├── main.rs         # plugin entry point + commands
    │       ├── protocol.rs     # TRAMP URI parser
    │       ├── errors.rs       # error types
    │       ├── vfs.rs          # VFS layer (caching, connection pool)
    │       ├── completion.rs   # tab completion for remote paths
    │       └── backend/
    │           ├── mod.rs      # Backend trait + registry
    │           ├── ssh.rs      # SSH backend (SFTP + exec)
    │           ├── exec.rs     # Docker / K8s / sudo backends
    │           ├── runner.rs   # CommandRunner abstraction (local/remote)
    │           ├── rpc.rs      # RPC backend (agent communication)
    │           ├── rpc_client.rs # MsgPack-RPC client (wire protocol)
    │           ├── deploy.rs   # Agent deployment via SSH/SFTP
    │           ├── deploy_exec.rs # Agent deployment in containers
    │           └── socket.rs   # TCP/Unix socket transport
    └── agent/              # tramp-agent — lightweight RPC agent
        ├── Cargo.toml
        └── src/
            ├── main.rs         # agent entry point (stdin/stdout/TCP/Unix RPC)
            ├── rpc.rs          # MsgPack-RPC framing + message types
            └── ops/
                ├── mod.rs      # operation module index
                ├── file.rs     # file.stat, file.read, file.write, …
                ├── dir.rs      # dir.list, dir.create, dir.remove
                ├── process.rs  # process.run, process.start, …
                ├── pty.rs      # process.start_pty, process.resize (PTY)
                ├── system.rs   # system.info, system.getenv, system.statvfs
                ├── batch.rs    # batch (N ops in 1 round-trip)
                └── watch.rs    # watch.add, watch.remove, watch.list
```

Placed under `dev/nushell/plugin-tramp/` in the `noverby/noverby` monorepo root, alongside
existing projects such as `safety/oxidized/nixos/`, `safety/oxidized/systemd/`, etc.

---

## 3. Path Format

### 3.1 Basic URI

```text
/<backend>:<user>@<host>#<port>:<remote-path>
```

| Segment | Required | Example |
|---|---|---|
| `backend` | ✅ | `ssh`, `docker`, `k8s` |
| `user` | ❌ | `admin` |
| `host` | ✅ | `myvm`, `192.168.1.10` |
| `port` | ❌ | `2222` |
| `remote-path` | ✅ | `/etc/config` |

Examples:

```text
/ssh:myvm:/etc/config
/ssh:admin@myvm:/etc/config
/ssh:admin@myvm#2222:/etc/config
```

### 3.2 Chained URIs (Phase 2+)

Hops are separated by `|`:

```text
/ssh:jumpbox|ssh:myvm:/etc/config
/ssh:myvm|docker:mycontainer:/app/config.toml
/ssh:myvm|sudo:root:/etc/shadow
```

The parser must produce a `Vec<Hop>` to represent chains. Execution of
chains is deferred to Phase 2 but the type system must support it from day 1.

### 3.3 Parsed Types

```rust
pub struct TrampPath {
    pub hops: Vec<Hop>,
    pub remote_path: String,
}

pub struct Hop {
    pub backend: BackendKind,
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

pub enum BackendKind {
    Ssh,
    Docker,
    Kubernetes,
    Sudo,
}
```

---

## 4. Architecture

```text
Nushell command
      │
      ▼
┌────────────���────────────────────────────┐
│        nu-plugin-tramp plugin           │
│                                         │
│  ┌─────────────┐   ┌─────────────────┐  │
│  │ Path Parser │──▶│ Backend Resolver│  │
│  └─────────────┘   └───────┬─────────┘  │
│                            │            │
│                   ┌────────▼────────┐   │
│                   │   VFS Layer     │   │
│                   └────────┬────────┘   │
│                            │            │
│              ┌─────────────┼──────┐     │
│              ▼             ▼      ▼     │
│           ┌─────┐    ┌────────┐  ...   │
│           │ SSH │    │ Docker │        │
│           └─────┘    └────────┘        │
└─────────────────────────────────────────┘
```

### 4.1 Layer 1 — Path Parser (`src/protocol.rs`)

- Parses a string into `TrampPath`
- Must detect whether a path is a tramp URI (starts with `/<known-backend>:`)
- Returns `None` / `Ok(None)` for non-tramp paths so Nushell handles them normally
- Must round-trip: `parse(format(path)) == path`

### 4.2 Layer 2 — Backend Trait (`src/backend/mod.rs`)

```rust
#[async_trait]
pub trait Backend: Send + Sync {
    async fn read(&self, path: &str) -> Result<Bytes>;
    async fn write(&self, path: &str, data: Bytes) -> Result<()>;
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>>;
    async fn stat(&self, path: &str) -> Result<Metadata>;
    async fn exec(&self, cmd: &str, args: &[&str]) -> Result<ExecResult>;
    async fn delete(&self, path: &str) -> Result<()>;
}

pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,      // File | Dir | Symlink
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub permissions: Option<u32>,
}

pub struct Metadata {
    pub kind: EntryKind,
    pub size: u64,
    pub modified: SystemTime,
    pub permissions: u32,
}

pub struct ExecResult {
    pub stdout: Bytes,
    pub stderr: Bytes,
    pub exit_code: i32,
}
```

### 4.3 Layer 3 — VFS (`src/vfs.rs`)

Responsibilities:

- Resolve a `TrampPath` to a concrete `Backend` instance
- Connection pooling: reuse open connections keyed by `(backend, user, host, port)`
- Optional: small stat cache with TTL (Phase 2)
- Optional: streaming for large files (Phase 2)
- For Phase 1: single-hop only; log a warning and return an error for chained paths

### 4.4 Layer 4 — SSH Backend (`src/backend/ssh.rs`)

Use the [`openssh`](https://crates.io/crates/openssh) crate (shells out to
system OpenSSH). This gives:

- Full `~/.ssh/config` support
- SSH agent forwarding
- `ControlMaster` multiplexing (fast subsequent ops)
- Key management delegated entirely to the user's existing setup

Operations:

| Operation | Implementation |
|---|---|
| `read` | `sftp.read(path)` or `exec cat <path>` |
| `write` | `sftp.write(path, data)` |
| `list` | `sftp.read_dir(path)` |
| `stat` | `sftp.metadata(path)` |
| `exec` | `session.command(cmd).args(args).output()` |
| `delete` | `sftp.remove_file(path)` or `exec rm <path>` |

Prefer SFTP subsystem where available; fall back to `exec` for compatibility.

---

## 5. Nushell Plugin Interface

The plugin is registered with Nushell via:

```nushell
plugin add ~/.cargo/bin/nu_plugin_tramp
plugin use tramp
```

It hooks into Nushell's built-in commands by intercepting path arguments that
match the TRAMP URI pattern.

### 5.1 Command Hooks

| Nushell command | Plugin behavior |
|---|---|
| `open /ssh:…` | Read remote file; return as Nushell value (auto-detect format: JSON, TOML, CSV, raw) |
| `save /ssh:…` | Serialise piped Nushell value; write to remote |
| `ls /ssh:…` | List remote directory; return as Nushell table |
| `cd /ssh:…` | Set `$env.PWD` to the tramp URI; resolve relative paths against it |
| `rm /ssh:…` | Delete remote file |
| `cp /ssh:… /ssh:…` | Read from source backend; write to destination backend |

### 5.2 Custom Commands (supplementary)

```nushell
# Explicit connection test
tramp ping /ssh:myvm:/

# Show active connections
tramp connections

# Disconnect
tramp disconnect myvm

# Interactive remote shell (requires RPC agent)
tramp shell /ssh:myvm:/
tramp shell /ssh:myvm:/app --shell /bin/bash
tramp shell /docker:mycontainer:/
```

### 5.3 `cd` Semantics

When `$env.PWD` is a TRAMP URI, any relative path passed to a TRAMP-aware
command is resolved against it:

```nushell
cd /ssh:myvm:/app
open config.toml     # resolves to /ssh:myvm:/app/config.toml
ls                   # lists /app on myvm
```

Non-TRAMP commands (e.g. `git`) receive `$env.PWD` as-is and will fail
gracefully — this is expected and documented.

---

## 6. Error Handling

All errors must implement `std::error::Error` and produce clear, user-facing
messages. Use `thiserror`.

| Error kind | Message example |
|---|---|
| Parse error | `invalid tramp path: missing remote path after ':'` |
| Connection failed | `ssh: could not connect to myvm: connection refused` |
| Auth failed | `ssh: authentication failed for admin@myvm` |
| File not found | `remote: no such file or directory: /etc/missing` |
| Permission denied | `remote: permission denied: /etc/shadow` |
| Chained path (v1) | `tramp: chained paths not yet supported (see roadmap)` |

---

## 7. Dependencies

```toml
[dependencies]
# Nushell plugin API
nu-plugin    = "0.101"
nu-protocol  = "0.101"

# SSH transport
openssh      = { version = "0.10", features = ["native-mux"] }
openssh-sftp-client = "0.14"

# Async runtime
tokio        = { version = "1", features = ["full"] }

# Utilities
bytes        = "1"
thiserror    = "2"
async-trait  = "0.1"
```

Pin `nu-plugin` / `nu-protocol` to the same version as the Nushell binary in
use. Nushell's plugin ABI is version-locked.

---

## 8. Nix Integration

Because the monorepo uses Nix + Crane, `nu-plugin-tramp` should expose:

```nix
# flake.nix addition (nu-plugin-tramp/default.nix)
{ crane, rust-overlay, ... }:
crane.buildPackage {
  src = ./.;
  pname = "nu-plugin-tramp";
}
```

And optionally a Home Manager module that auto-registers the plugin:

```nix
programs.nushell.plugins = [ pkgs.nu-plugin-tramp ];
```

---

## 9. Phased Roadmap

### Phase 1 — MVP (initial PR)

- [x] TRAMP URI parser (single-hop SSH only)
- [x] SSH backend via `openssh` + `openssh-sftp-client`
- [x] `open`, `ls`, `save` working end-to-end
- [x] `rm` support
- [x] Nushell plugin compiles and registers correctly
- [x] README with install + usage
- [x] Nix package derivation (`nu-plugin-tramp/default.nix`)

### Phase 2 — Daily Driver

- [x] Connection pooling + keepalive
- [x] `cd` with relative path resolution
- [x] Stat + small file cache (with TTL)
- [x] Streaming for large files
- [x] `cp` between remotes
- [x] `tramp ping/connections/disconnect` commands

### Phase 3 — Power Features

- [x] Path chaining (SSH → Docker, SSH → Sudo, triple chains, etc.)
- [x] Docker backend
- [x] Kubernetes (`kubectl exec`) backend
- [x] `sudo` backend
- [x] Push execution model (`tramp exec` command)

### Phase 4 — Polish & Usability

- [x] `tramp info` command — show remote system info (OS, arch, hostname, user, disk usage) in one round-trip
- [x] Richer `DirEntry` metadata — inode, nlinks, uid/gid, owner/group names, symlink targets
- [x] Batch stat in listings — combine `stat` calls for all entries in a directory into a single remote command
- [x] Configurable cache TTL via `$env.TRAMP_CACHE_TTL` environment variable
- [x] Home Manager module for auto-registration (`hm-module.nix`)
- [x] Glob/wildcard support for `tramp ls --glob` and `tramp cp --glob`
- [x] Tab completion for remote paths

### Phase 5 — RPC Agent (inspired by [emacs-tramp-rpc](https://github.com/ArthurHeymans/emacs-tramp-rpc))

The biggest performance opportunity is replacing shell-command-parsing with a
lightweight **RPC agent** deployed on the remote host. `emacs-tramp-rpc`
demonstrates 2–38× speedups over traditional shell-based TRAMP by using this
approach.

#### 5.1 — `tramp-agent` binary ✅

A small, statically-linked Rust binary (~1 MB) that runs on the remote host
and speaks a length-prefixed MessagePack-RPC protocol over stdin/stdout
(piped through the SSH connection).

```text
┌──────────────────┐  SSH stdin/stdout  ┌───────────────────┐
│  nu-plugin-tramp │ ◄───────────────► │   tramp-agent     │
│  (local Nushell) │   MsgPack-RPC     │   (remote Rust)   │
└──────────────────┘                    └───────────────────┘
```

RPC methods exposed by the agent:

| Category   | Methods                                                        |
|------------|----------------------------------------------------------------|
| File       | `file.stat`, `file.stat_batch`, `file.truename`, `file.size`  |
| File I/O   | `file.read`, `file.read_range`, `file.write`,                 |
|            | `file.write_range`, `file.copy`, `file.rename`,               |
|            | `file.delete`, `file.set_modes`                               |
| Directory  | `dir.list`, `dir.create`, `dir.remove`                        |
| Process    | `process.run`, `process.start`, `process.read`,               |
|            | `process.write`, `process.kill`                               |
| System     | `system.info`, `system.getenv`, `system.statvfs`              |
| Batch      | `batch` (multiple ops in one round-trip)                       |
| Watch      | `watch.add`, `watch.remove`, `watch.list`                     |

Key advantages over shell parsing:

| Aspect             | Current (shell parsing)        | With tramp-agent              |
|--------------------|-------------------------------|-------------------------------|
| Communication      | Individual SSH exec commands   | MsgPack-RPC over single pipe  |
| Latency            | N operations = N round-trips   | N operations = 1 round-trip   |
| Binary data        | base64 encode/decode           | Native binary (MsgPack bin)   |
| Stat + list        | Separate commands, text parse  | Native `lstat()` syscalls     |
| Shell dependency   | Requires sh, stat, cat, etc.  | None (self-contained binary)  |
| Cache invalidation | TTL-based (5s default)         | inotify/kqueue push events    |

#### 5.2 — Automatic deployment ✅

On first connection (when the agent is not yet present on the remote), the
plugin:

1. Detects the remote OS and architecture (`uname -sm`)
2. Checks a local cache (`~/.cache/nu-plugin-tramp/VERSION/ARCH/tramp-agent`)
3. Downloads a pre-built binary from GitHub Releases (or builds from source
   if Rust is installed on the remote)
4. Uploads via SFTP to `~/.cache/tramp-agent/tramp-agent` on the remote
5. Starts the agent and switches the backend to RPC mode

If deployment fails, the plugin falls back to the current shell-parsing
approach transparently — no user action required.

```text
Connect via SSH
       │
       ▼
Agent already deployed? ──yes──► Start agent, use RPC
       │ no
       ▼
Detect remote arch (uname -sm)
       │
       ▼
Check local cache
       │
       ├─ Found ──────────────► Upload via SFTP, start
       │
       ▼
Download from GitHub Releases
       │
       ├─ Success ────────────► Cache locally, upload, start
       │
       ▼
Build with cargo (if available)
       │
       ├─ Success ────────────► Cache locally, upload, start
       │
       ▼
Fall back to shell-parsing mode (current behavior)
```

#### 5.3 — Filesystem watching (push cache invalidation) ✅

When the agent is running, it uses `inotify` (Linux) or `kqueue` (macOS) to
watch directories that the client has recently accessed. On filesystem
changes, the agent sends an `fs.changed` notification with the affected
paths. The VFS layer uses these to invalidate specific cache entries
immediately, rather than waiting for TTL expiry.

This is especially valuable for workflows where an editor and shell are both
operating on the same remote directory — changes are reflected instantly.

#### 5.4 — Batch and parallel operations ✅

The `batch` RPC method allows the client to send multiple operations in a
single request:

```text
Client sends:
  batch { requests: [
    { method: "file.stat", params: { path: "/app/config.toml" } },
    { method: "file.stat", params: { path: "/app/data.json" } },
    { method: "dir.list",  params: { path: "/app" } },
  ]}

Agent responds:
  { results: [ {result: ...}, {result: ...}, {result: ...} ] }
```

The `commands.run_parallel` method runs multiple commands concurrently using
OS threads (inspired by emacs-tramp-rpc's magit optimization that sends ~60
git commands in a single round-trip).

#### 5.5 — Agent for exec backends ✅

The agent concept extends beyond SSH. For Docker and Kubernetes backends,
the agent binary can be copied into containers (`docker cp` / `kubectl cp`)
and started via `docker exec` / `kubectl exec`, giving the same RPC
performance benefits inside containers.

Implementation (`src/backend/deploy_exec.rs`):

- **Architecture detection**: runs `uname -sm` inside the container via the
  appropriate exec prefix (`docker exec` / `kubectl exec`)
- **Upload methods**:
  - Docker: `docker cp <local_tmp> <container>:/tmp/tramp-agent-dir/tramp-agent`
  - Kubernetes: `kubectl cp` with base64 exec fallback (for minimal containers
    lacking `tar`)
  - Docker base64 fallback for remote runners
- **Agent startup**: spawns `docker exec -i` / `kubectl exec -i` as an
  interactive `tokio::process::Child` with piped stdin/stdout
- **VFS integration**: `connect_hop` attempts agent deployment for standalone
  Docker/K8s containers (no parent backend); on failure, falls back to the
  shell-parsing `ExecBackend` transparently
- **Chained paths**: for `/ssh:host|docker:ctr:/path`, the parent SSH hop
  already uses the RPC agent, so Docker commands routed through it are
  already fast; deploying a second agent inside the container through a
  remote runner is deferred to Phase 6

```text
Standalone Docker/K8s:
  ┌──────────────┐  docker exec -i  ┌───────────────────┐
  │  nu-plugin-   │ ◄──────────────► │   tramp-agent     │
  │  tramp        │   MsgPack-RPC    │   (in container)  │
  └──────────────┘                   └───────────────────┘

Chained (SSH → Docker):
  ┌──────────────┐  SSH pipe   ┌─────────┐  docker exec  ┌───────────┐
  │  nu-plugin-   │ ◄────────► │  agent   │ ◄──────────► │ container │
  │  tramp        │  MsgPack   │ (remote) │  shell cmds   │           │
  └──────────────┘             └─────────┘               └───────────┘
```

#### 5.6 — Protocol design ✅

- **Framing**: 4-byte big-endian length prefix + MessagePack payload
- **Request**: `{ version: "2.0", id: N, method: "...", params: {...} }`
- **Response**: `{ version: "2.0", id: N, result: ... }` or `{ ..., error: { code, message } }`
- **Notification** (server → client): `{ version: "2.0", method: "fs.changed", params: { paths: [...] } }`
- **Binary data**: MessagePack `bin` type for file content (no base64)
- **Concurrent requests**: Multiple requests can be in-flight; responses are matched by `id`

Dependencies for the agent binary:

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
rmp-serde = "1.3"
rmpv = { version = "1.3", features = ["with-serde"] }
tokio = { version = "1", features = ["rt-multi-thread", "io-util", "io-std", "fs", "process", "sync"] }
notify = "6.1"     # inotify/kqueue
libc = "0.2"       # for native stat, uid/gid resolution
```

Build profile for minimal binary size:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

### Phase 4.1 — Tab Completion for Remote Paths ✅

Dynamic tab completion is implemented via `get_dynamic_completion` on every
plugin command that accepts a TRAMP path argument (`src/completion.rs`).

Completion stages:

| User input            | Completions offered                              |
|-----------------------|--------------------------------------------------|
| `/`                   | Backend prefixes: `/ssh:`, `/docker:`, `/k8s:`, `/sudo:` |
| `/ss`                 | `/ssh:`                                          |
| `/ssh:`               | Host names from `~/.ssh/config` + active VFS connections |
| `/docker:`            | Running container names (`docker ps`)            |
| `/k8s:`               | Pod names (`kubectl get pods`)                   |
| `/ssh:host:`          | Suggest `/ssh:host:/` to start remote path       |
| `/ssh:host:/etc/`     | Remote directory listing of `/etc/`              |
| `/ssh:host:/etc/ho`   | Filtered entries starting with `ho`              |
| `subdir/` (with CWD)  | Relative path listing against remote CWD        |

Architecture additions:

- **`completion.rs`** — new module with all completion logic:
  - `complete_tramp_path()` — main entry point, dispatches to stage-specific
    completers based on how much of the URI has been typed
  - `extract_positional_string()` — extracts the partial argument text from
    the AST `Call`, handling the `strip` placeholder flag
  - `complete_backend_prefix()` — prefix-matches known backend names
  - `complete_host()` — gathers host suggestions from SSH config, active
    connections, `docker ps`, and `kubectl get pods`
  - `complete_remote_path()` — lists the parent directory via VFS and filters
    by the partial filename; directories get a trailing `/`
  - `complete_relative_path()` — relative path completion when a remote CWD
    is active via `tramp cd`
  - `parse_ssh_config_hosts()` — parses `~/.ssh/config` `Host` entries
    (skipping wildcards)
  - `list_docker_containers()` / `list_k8s_pods()` — best-effort subprocess
    calls for container/pod name discovery
- **`complete_tramp_arg()`** — shared helper in `main.rs` that extracts the
  positional argument and calls `complete_tramp_path()`
- **All path-accepting commands** implement `get_dynamic_completion`:
  `tramp open`, `tramp ls`, `tramp save`, `tramp rm`, `tramp cp` (both
  source and destination), `tramp cd`, `tramp exec`, `tramp info`,
  `tramp ping`, `tramp watch`

### Phase 6 — Future ✅

- [x] Streaming for very large files (chunked RPC reads/writes)
- [x] PTY support via agent (remote terminal emulation)
- [x] `tramp watch` command — subscribe to filesystem change notifications
- [x] Agent version management and auto-upgrade
- [x] TCP/Unix socket transport for agent (local Docker without SSH)
- [x] Cross-compilation matrix in CI for agent binaries (x86_64/aarch64 × Linux/macOS)

#### Streaming for very large files ✅

Chunked I/O is implemented via three new agent RPC methods and corresponding
`Backend` trait methods, enabling streaming reads and writes of arbitrarily
large files without loading them entirely into memory.

New agent RPC methods:

| Method             | Description                                      |
|--------------------|--------------------------------------------------|
| `file.size`        | Get file size (lightweight, no full stat)         |
| `file.read_range`  | Read a byte range: `{ path, offset, length }`    |
| `file.write_range` | Write at offset: `{ path, offset, data, truncate, create }` |

`file.read_range` returns `{ data: <binary>, eof: <bool> }` — the client
reads in a loop, advancing `offset` by the chunk size each iteration, until
`eof` is `true`.

`file.write_range` supports chunked uploads: send the first chunk with
`truncate: true` to create/truncate the file, then subsequent chunks with
increasing offsets. An explicit `flush()` after each `write_all` ensures
data reaches disk before the success response is sent.

Architecture additions:

- **`Backend` trait** — new optional methods with default fallback
  implementations:
  - `file_size(path)` — defaults to `stat(path).size`
  - `read_range(path, offset, length)` — defaults to reading the whole file
    and slicing (defeats streaming, but works)
  - `write_range(path, offset, data, truncate)` — defaults to whole-file
    write (only correct for offset=0 + truncate)
  - `supports_streaming()` — returns `false` by default
- **`RpcBackend`** — implements all streaming methods natively via the agent
  RPC calls; `supports_streaming()` returns `true`
- **`VFS`** — new public methods: `file_size`, `read_range`, `write_range`,
  `supports_streaming`
- **`tramp open`** — files larger than 1 MB on streaming-capable backends
  are returned as a `ByteStream` (via `ByteStream::from_fn`) instead of a
  single `Value`, enabling pipelines like
  `tramp open /ssh:host:/big.bin | save local.bin` without OOM
- **`tramp save`** — `ByteStream` input on streaming-capable backends is
  written in 1 MB chunks via `write_range`, avoiding buffering the entire
  payload in memory; non-streaming backends fall back to collecting the
  stream first

Chunk size: 1 MB (read and write). Agent caps `file.read_range` at 16 MB
per request for safety.

#### `tramp watch` command ✅

Implemented as a plugin command that leverages the RPC agent's
`watch.add` / `watch.remove` / `watch.list` methods and `fs.changed`
push notifications (inotify/kqueue).

```text
tramp watch /ssh:myvm:/app --duration 5000      # watch for 5 seconds
tramp watch /ssh:myvm:/app --recursive           # include subdirectories
tramp watch /ssh:myvm:/ --list                   # show active watches
tramp watch /ssh:myvm:/app --remove              # stop watching
```

Architecture additions:

- **`Backend` trait** — new optional methods: `watch_add`, `watch_remove`,
  `watch_list`, `watch_poll`, `supports_watch` (default: not supported)
- **`RpcBackend`** — implements all watch methods by calling the agent's
  RPC endpoints and draining buffered `fs.changed` notifications
- **`VFS`** — exposes synchronous `watch_add`, `watch_remove`, `watch_list`,
  `watch_poll`, `supports_watch` methods
- **`tramp watch` command** — adds a watch, polls for events over the
  specified duration (default 10s, 250ms poll interval), returns a table
  of `{ paths, kind, timestamp }` records, then cleans up the watch

#### Agent version management ✅

The agent binary now supports `--version` / `-V` flags, printing
`tramp-agent <version>` to stdout.

The deployment module (`deploy.rs` and `deploy_exec.rs`) checks the
remote agent's version by running `<agent> --version` during the
`is_agent_deployed` / `is_agent_deployed_in_container` checks. If the
version doesn't match the plugin's `CARGO_PKG_VERSION`, the agent is
re-uploaded and restarted automatically — ensuring plugin and agent are
always in sync after upgrades.

---

#### TCP/Unix socket transport for agent ✅

The agent now supports a `--listen` flag that makes it listen on a TCP port
or Unix domain socket instead of stdin/stdout. This enables direct socket
connections from the plugin, bypassing the overhead of a `docker exec -i` or
`kubectl exec -i` process sitting in the middle of every RPC exchange.

Usage:

```text
# TCP listener (port 0 = ephemeral):
tramp-agent --listen tcp:0.0.0.0:9547
tramp-agent --listen tcp:127.0.0.1:0

# Unix socket listener:
tramp-agent --listen unix:/tmp/tramp-agent.sock

# Formats also accepted without scheme prefix:
tramp-agent --listen 127.0.0.1:9547
tramp-agent --listen /tmp/tramp-agent.sock    # (Unix only)
```

The agent prints a machine-readable `LISTEN:tcp:<addr>` or
`LISTEN:unix:<path>` line to stderr so the plugin can discover the actual
bound address (especially useful with ephemeral ports).

Architecture additions:

- **Agent (`main.rs`)**:
  - `parse_listen_addr()` — parses `--listen` argument into `ListenAddr::Tcp`
    or `ListenAddr::Unix`
  - `serve_connection()` — transport-agnostic request loop extracted from the
    old `main()`, generic over `AsyncRead + AsyncWrite`
  - `run_tcp_listener()` — binds a TCP listener and accepts connections
    sequentially
  - `run_unix_listener()` — binds a Unix domain socket listener with cleanup
    on shutdown
  - Default mode (no `--listen`) serves a single connection over stdin/stdout
    as before

- **Plugin (`backend/socket.rs`)** — new module:
  - `SocketAddr` enum — parsed TCP or Unix address
  - `parse_socket_addr()` — mirrors the agent's address parser
  - `connect_tcp()` / `connect_unix()` — establish a connection, create an
    `RpcClient`, verify with a ping, and return an `RpcBackend`
  - `connect()` — generic dispatcher
  - `start_docker_tcp_agent()` — deploys agent in a Docker container with
    `--listen tcp:...`, discovers the container IP via `docker inspect`,
    and connects directly via TCP
  - `start_k8s_tcp_agent()` — deploys agent in a Kubernetes pod, sets up
    `kubectl port-forward`, and connects via the forwarded local port
  - `get_docker_container_ip()` — helper to query a container's network IP

- **VFS (`vfs.rs`)** — `connect_hop` for standalone Docker and Kubernetes
  now attempts a TCP socket upgrade after deploying the agent, falling back
  to the pipe-based RPC backend if TCP is unavailable

Key benefits:

| Aspect               | stdin/stdout pipe       | TCP/Unix socket            |
|----------------------|------------------------|----------------------------|
| Intermediate process | `docker exec -i` alive | None (direct connection)   |
| Connection lifetime  | Tied to exec process   | Independent / persistent   |
| Reconnection         | Must restart exec      | Just reconnect the socket  |
| Multiple clients     | One per exec process   | Sequential accept loop     |
| Latency overhead     | Docker/kubectl proxy   | Direct TCP or Unix IPC     |

#### Cross-compilation matrix for agent binaries ✅

A Nix-based cross-compilation infrastructure is provided in `cross.nix` for
building the `tramp-agent` binary for multiple target architectures from a
single host. Linux targets are statically linked via musl for maximum
portability — the resulting binaries can be deployed to any Linux host
regardless of its libc.

Supported targets:

| Target              | Triple                        | Notes                    |
|---------------------|-------------------------------|--------------------------|
| x86_64-linux        | x86_64-unknown-linux-musl     | Most servers & desktops  |
| aarch64-linux       | aarch64-unknown-linux-musl    | ARM64 (Raspberry Pi 4+) |
| x86_64-darwin       | x86_64-apple-darwin           | Intel Mac                |
| aarch64-darwin      | aarch64-apple-darwin          | Apple Silicon Mac        |

Usage:

```nix
# Build all Linux agent binaries from an x86_64-linux host:
cross = import ./cross.nix { inherit nixpkgs; };
agents = cross.allLinuxFrom "x86_64-linux";
# → agents.x86_64-linux  (statically linked musl binary)
# → agents.aarch64-linux (cross-compiled, statically linked)
```

The `cacheLayout` helper produces a directory tree matching the plugin's
deployment module expectations:

```text
$out/
├── x86_64-unknown-linux-musl/
│   └── tramp-agent
└── aarch64-unknown-linux-musl/
    └── tramp-agent
```

This can be copied into `~/.cache/nu-plugin-tramp/<version>/` so the
plugin's automatic deployment picks up the correct binary for each remote
host's architecture.

Architecture additions:

- **`cross.nix`** — new module:
  - `buildAgent` / `buildAgentStatic` — derivation builders for native
    and musl-static targets
  - `getCrossPkgs` — helper to set up Nix cross-compilation pkgs
  - `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `aarch64-darwin` —
    per-target derivations
  - `allLinuxFrom` — build all Linux targets from a given host
  - `matrix` — structured list of all targets for CI iteration
  - `cacheLayout` — produces the local cache directory structure
- **`default.nix`** — new `tramp-agent-cache` package that produces the
  cache layout with both Linux architectures

---

#### `tramp shell` command ✅

Interactive remote terminal session leveraging the PTY support built into
the agent, providing a seamless shell-in-shell experience without leaving
Nushell.

```nushell
tramp shell /ssh:myvm:/                    # default shell on remote
tramp shell /ssh:myvm:/app --shell /bin/bash  # specific shell + CWD
tramp shell /docker:mycontainer:/          # shell inside a container
tramp shell /ssh:myvm|docker:ctr:/         # chained: SSH → Docker
```

The command opens `/dev/tty` directly (bypassing the plugin's stdin/stdout
MsgPack-RPC channel) and puts it in raw mode, then relays bytes
bidirectionally between the local terminal and the remote PTY.

Architecture:

- **Terminal I/O (`tty_raw` module in `main.rs`)** — minimal libc-based
  terminal helpers:
  - `open_tty()` — opens `/dev/tty` for read+write
  - `terminal_size()` — queries `TIOCGWINSZ` for current cols×rows
  - `enable_raw_mode()` / `restore_mode()` — save/restore termios,
    `cfmakeraw` for raw mode
  - `read_with_timeout()` — `poll(2)` + `read(2)` with millisecond timeout
  - `write_all()` — blocking write loop via `libc::write`
  - `RawModeGuard` — RAII guard that restores termios on drop (panic-safe)

- **`TrampShell` command** (`#[cfg(unix)]`):
  - Checks `supports_pty()` before proceeding
  - Detects remote default shell via `echo $SHELL` (overridable with
    `--shell` flag)
  - Sets remote CWD from the URI's path component via `cd <dir> && exec`
  - Starts a PTY with the detected terminal dimensions
  - Spawns an **input thread** (`/dev/tty` → `pty_write`) with 50ms poll
    timeout
  - Runs an **output loop** on the main thread (`pty_read` → `/dev/tty`
    write) with 5ms idle sleep
  - Periodically checks for terminal resize (~every 100ms) and sends
    `pty_resize` when dimensions change
  - On session end (remote shell exits), signals the input thread via
    `AtomicBool`, restores the terminal, and kills the PTY

- **Dependencies** — adds `libc = "0.2"` to the plugin crate for raw
  terminal manipulation (already used by the agent crate)

Key design decisions:

| Aspect                | Approach                                         |
|-----------------------|--------------------------------------------------|
| Terminal access       | `/dev/tty` (bypasses plugin MsgPack-RPC stdio)   |
| Raw mode              | `cfmakeraw` + RAII guard for safe restore        |
| Input forwarding      | Dedicated thread with `poll(2)` + 50ms timeout   |
| Output forwarding     | Main thread loop with 5ms idle sleep             |
| Terminal resize       | Periodic size check (~100ms) + `pty_resize` RPC  |
| CWD support           | `cd <dir> && exec <shell>` wrapper               |
| Platform              | Unix only (`#[cfg(unix)]`)                       |

---

#### PTY support via agent ✅

The agent now supports pseudo-terminal (PTY) allocation for running
interactive commands on the remote host. This enables proper TTY-aware
behaviour (e.g. colour output, line editing, signal handling) for commands
executed through the RPC agent.

New agent RPC methods:

| Method              | Description                                        |
|---------------------|----------------------------------------------------|
| `process.start_pty` | Start a process with a PTY, return handle + PID    |
| `process.resize`    | Send a window size change (TIOCSWINSZ + SIGWINCH)  |

PTY processes reuse the existing `process.read`, `process.write`, and
`process.kill` methods — the dispatch layer routes to the correct table
based on handle range (PTY handles ≥ 1,000,000).

`process.start_pty` parameters:

- `program` (string, required) — the program to execute
- `args` (array of strings, optional) — command arguments
- `cwd` (string, optional) — working directory
- `env` (map of string→string, optional) — environment variables
- `rows` (u16, optional, default 24) — initial terminal rows
- `cols` (u16, optional, default 80) — initial terminal columns

Architecture additions:

- **Agent (`ops/pty.rs`)** — new module implementing PTY support:
  - `openpty()` — creates a master/slave PTY pair via `libc::openpty`
  - `set_winsize()` — sets terminal dimensions via `TIOCSWINSZ` ioctl
  - `AsyncPtyMaster` — tokio `AsyncFd` wrapper for non-blocking I/O on
    the PTY master fd
  - `fork_pty()` — synchronous helper that allocates the PTY, forks, sets
    `setsid()` + `TIOCSCTTY`, redirects stdio to the slave PTY, and execs
    the program — all non-`Send` raw pointer work is confined here
  - `PtyTable` — shared table of managed PTY processes (separate from the
    regular `ProcessTable` to avoid handle collisions)
  - `is_pty_handle()` — checks if a handle ID belongs to the PTY table
  - Full read/write/kill/resize handlers with proper child reaping
    (`waitpid`) and EIO handling (slave closed on child exit)
  - Platform-gated: full implementation on Unix, stub errors on non-Unix

- **Agent (`main.rs`)** — dispatch updated:
  - `process.start_pty` → `ops::pty::start_pty`
  - `process.resize` → `ops::pty::resize`
  - `process.read` / `process.write` / `process.kill` auto-route to
    `ops::pty` when the handle is in the PTY range

- **Plugin (`backend/mod.rs`)** — `Backend` trait additions:
  - `PtyHandle` and `PtyReadResult` types
  - Optional methods: `pty_start`, `pty_read`, `pty_write`, `pty_resize`,
    `pty_kill`, `supports_pty` (default: not supported)

- **Plugin (`backend/rpc.rs`)** — `RpcBackend` implements all PTY methods
  natively via the agent RPC calls; `supports_pty()` returns `true`

- **Plugin (`vfs.rs`)** — new public methods: `supports_pty`, `pty_start`,
  `pty_read`, `pty_write`, `pty_resize`, `pty_kill`

---

## 10. Non-Goals (v1)

- GUI or TUI
- Windows support (SFTP client may not build; untested)
- Non-Nushell shells
- Encrypted secrets management (delegate to `agenix`/`ragenix` already in the monorepo)
- FUSE mount (use SSHFS if you need that separately)

---

## 11. Future Directions (Beyond v1)

With all v1 roadmap phases complete, potential areas for future work include:

- **Chained-path agent nesting** — deploy a second `tramp-agent` *inside* a
  container reached via a chained path (e.g. `/ssh:host|docker:ctr:/path`),
  giving full RPC performance even for the inner hop (currently the inner
  hop falls back to shell-parsing through the outer agent)
- **GitHub Releases CI** — automated release pipeline that builds the
  cross-compilation matrix and publishes agent binaries to GitHub Releases
  for automatic download during deployment
- **FUSE integration** — optional FUSE mount mode for applications that
  expect a real filesystem path (complementary to the current VFS approach)
- **Windows agent support** — extend PTY and socket transport to Windows
  targets (ConPTY, named pipes)
- **Plugin marketplace** — package for Nushell's future plugin registry
  once it materialises