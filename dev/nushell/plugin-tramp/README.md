# nu-plugin-tramp

> A TRAMP-inspired remote filesystem plugin for [Nushell](https://www.nushell.sh/), written in Rust.

The key design principle — inherited from Emacs TRAMP — is that **the shell
and all its tools stay local**. Only file I/O crosses the transport boundary.
Nushell's structured data model makes this a natural fit: `tramp open` returns
a typed Nushell value; pipes operate locally; `tramp save` writes back remotely.

## Features

- **TRAMP-style URIs** — `/ssh:user@host#port:/remote/path`
- **Multiple backends** — SSH, Docker, Kubernetes, and Sudo
- **Path chaining** — reach nested environments: `/ssh:myvm|docker:ctr:/app/config.toml`
- **Full `~/.ssh/config` support** — host aliases, keys, jump hosts all work
- **SSH agent forwarding** — no extra configuration needed
- **ControlMaster multiplexing** — fast subsequent operations on the same host
- **Connection pooling with health-checks** — sessions are reused across commands; stale connections are detected and transparently reconnected
- **SFTP fast-path** — file read/write/delete uses the SFTP subsystem for efficient binary-safe transfer without base64 overhead or shell argument limits; falls back to exec transparently if SFTP is unavailable
- **Rich metadata** — `tramp ls` shows owner, group, nlinks, inode, and symlink targets (all gathered in a single remote command via batch-stat)
- **Stat & directory listing cache** — metadata is cached with a configurable TTL (default 5s) to avoid redundant remote round-trips
- **Configurable cache TTL** — set `$env.TRAMP_CACHE_TTL` to tune caching (supports durations like `10sec`, `500ms`, or `0` to disable)
- **Glob/wildcard filtering** — `tramp ls --glob '*.log'` and `tramp cp --glob '*.conf'` for pattern-based operations
- **Remote working directory** — `tramp cd` lets you navigate remote hosts with relative paths
- **Cross-host copy** — `tramp cp` transfers files between any combination of local and remote paths
- **Push execution** — `tramp exec` runs arbitrary commands on the remote
- **Tab completion** — dynamic completions for TRAMP paths: backend prefixes, host names (from `~/.ssh/config`, `docker ps`, `kubectl get pods`, active connections), and remote file/directory listings
- **Streaming large files** — files over 1 MB are read/written in chunks via the RPC agent's `file.read_range` / `file.write_range` methods, enabling `tramp open /ssh:host:/big.bin | save local.bin` without loading the entire file into memory
- **PTY support** — the RPC agent can allocate pseudo-terminals for interactive commands via `process.start_pty` and `process.resize`, enabling proper TTY-aware behaviour (colour output, line editing, signal handling)
- **Interactive remote shell** — `tramp shell` opens a full interactive terminal session on a remote host via the RPC agent's PTY support, with automatic terminal resize detection, raw mode forwarding, and CWD support
- **Socket transport** — the agent supports `--listen tcp:<addr>:<port>` and `--listen unix:<path>` modes for direct TCP/Unix socket connections, bypassing `docker exec` overhead for local containers
- **Cross-compilation** — Nix-based build matrix (`cross.nix`) produces statically-linked musl agent binaries for x86_64 and aarch64 Linux, plus native Darwin builds for Intel and Apple Silicon Macs
- **Home Manager module** — auto-register the plugin with Nushell via `programs.nu-plugin-tramp.enable`

## Installation

### From source

```sh
cargo install --path .
```

### Register with Nushell

```nushell
plugin add ~/.cargo/bin/nu_plugin_tramp
plugin use tramp
```

### With Nix

The package is available as `nu-plugin-tramp` from the monorepo flake:

```sh
nix build .#nu-plugin-tramp
```

### With Home Manager

Import the Home Manager module for automatic registration:

```nix
# home.nix
{ inputs, ... }:
{
  imports = [ inputs.nu-plugin-tramp.homeManagerModules.default ];

  programs.nu-plugin-tramp = {
    enable = true;
    # cacheTTL = "10sec";  # optional: override the default 5s cache TTL
  };
}
```

## Usage

### Read a remote file

```nushell
# Read a text file
tramp open /ssh:myvm:/etc/hostname

# Read and parse a remote JSON file
tramp open /ssh:myvm:/app/config.json | from json

# Read a remote TOML config
tramp open /ssh:admin@myvm:/app/config.toml | from toml
```

### List a remote directory

```nushell
# List files in a remote directory (includes owner, group, nlinks, inode, symlink targets)
tramp ls /ssh:myvm:/var/log

# Filter to only directories
tramp ls /ssh:myvm:/ | where type == dir

# Sort by size
tramp ls /ssh:myvm:/var/log | sort-by size --reverse

# Filter by owner
tramp ls /ssh:myvm:/var/log | where owner == root

# Show only symlinks and their targets
tramp ls /ssh:myvm:/usr/lib | where type == symlink | select name target

# Glob/wildcard filtering — list only .log files
tramp ls /ssh:myvm:/var/log --glob '*.log'

# Match entries starting with "config"
tramp ls /ssh:myvm:/etc -g 'config*'
```

### Write to a remote file

```nushell
# Write a string
"hello world" | tramp save /ssh:myvm:/tmp/hello.txt

# Pipe structured data through a serialiser
open local-config.toml | to toml | tramp save /ssh:myvm:/app/config.toml

# Copy a local file to remote
open local-file.bin | tramp save /ssh:myvm:/tmp/remote-file.bin
```

### Delete a remote file

```nushell
tramp rm /ssh:myvm:/tmp/stale.lock
```

### Copy files

```nushell
# Remote → Local
tramp cp /ssh:myvm:/etc/hostname ./hostname

# Local → Remote
tramp cp ./config.toml /ssh:myvm:/app/config.toml

# Remote → Remote (even across different hosts)
tramp cp /ssh:vm1:/etc/config /ssh:vm2:/etc/config

# Glob copy — copy all .log files from remote to a local directory
tramp cp /ssh:myvm:/var/log ./logs --glob '*.log'

# Glob copy — copy matching files between remotes
tramp cp /ssh:vm1:/etc /ssh:vm2:/etc/backup --glob '*.conf'
```

### Execute remote commands

Run arbitrary commands on a remote target using push execution:

```nushell
# Run a command on a remote SSH host
tramp exec /ssh:myvm:/ -- ls -la /tmp

# Run inside a Docker container on a remote host
tramp exec /ssh:myvm|docker:ctr:/ -- hostname

# Run inside a local Docker container
tramp exec /docker:mycontainer:/ -- cat /etc/os-release

# Run as root via sudo
tramp exec /sudo:root:/ -- cat /etc/shadow
```

### Docker backend

Access files inside Docker containers, locally or through SSH:

```nushell
# Local Docker container
tramp open /docker:mycontainer:/app/config.toml
tramp ls /docker:mycontainer:/var/log

# Docker container on a remote host (chained)
tramp open /ssh:myvm|docker:webapp:/app/config.toml
tramp ls /ssh:myvm|docker:webapp:/var/log

# With a specific user inside the container
tramp open /docker:admin@mycontainer:/app/config.toml
```

### Kubernetes backend

Access files inside Kubernetes pods:

```nushell
# Access a pod (uses current kubectl context)
tramp open /k8s:mypod:/app/config.toml
tramp ls /k8s:mypod:/var/log

# Specify a container in a multi-container pod
tramp open /k8s:sidecar@mypod:/app/config.toml

# Pod on a remote host (chained through SSH)
tramp open /ssh:myvm|k8s:mypod:/app/config.toml
```

### Sudo backend

Access files as another user via sudo:

```nushell
# Read a root-only file
tramp open /sudo:root:/etc/shadow

# Sudo through SSH
tramp open /ssh:myvm|sudo:root:/etc/shadow

# List as another user
tramp ls /sudo:www-data:/var/www
```

### Path chaining

Chain multiple hops to reach nested environments. Each hop is separated by `|`:

```nushell
# SSH → Docker
tramp open /ssh:myvm|docker:webapp:/app/config.toml

# SSH → Sudo
tramp open /ssh:myvm|sudo:root:/etc/shadow

# SSH → Docker → Sudo (triple chain)
tramp ls /ssh:myvm|docker:webapp|sudo:root:/etc

# Use cd for repeated access
tramp cd /ssh:myvm|docker:webapp:/app
tramp open config.toml
tramp ls
```

### Remote working directory

Set a remote CWD so that subsequent commands can use relative paths:

```nushell
# Set the remote working directory
tramp cd /ssh:myvm:/app

# Now use relative paths — resolves to /ssh:myvm:/app/config.toml
tramp open config.toml

# List the current remote directory
tramp ls

# Navigate relatively
tramp cd subdir
tramp cd ..

# Show the current remote CWD
tramp pwd

# Clear the remote CWD
tramp cd --reset
```

Non-TRAMP commands (e.g. `git`) receive `$env.PWD` as-is and are unaffected —
only `tramp` subcommands participate in remote CWD resolution.

### System information

Gather remote system info in a single round-trip (inspired by emacs-tramp-rpc's
`system.info` RPC method):

```nushell
# Show OS, arch, hostname, user, kernel, CPU count, and disk usage
tramp info /ssh:myvm:/

# Works with any backend
tramp info /docker:mycontainer:/

# Works with chained paths
tramp info /ssh:myvm|docker:webapp:/
```

Example output:

```text
╭────────────────┬───────────────────────╮
│ os             │ Linux                 │
│ arch           │ x86_64                │
│ hostname       │ myvm                  │
│ user           │ admin                 │
│ kernel         │ 6.1.0-18-amd64       │
│ cpus           │ 4                     │
│ disk_total     │ 50.0 GiB             │
│ disk_used      │ 12.3 GiB             │
│ disk_available │ 35.2 GiB             │
│ disk_use_pct   │ 26%                   │
│ connection     │ /ssh:myvm:/           │
╰────────────────┴───────────────────────╯
```

### Connection management

```nushell
# Test connectivity to a remote host (reports timing)
tramp ping /ssh:myvm:/

# List all active pooled connections
tramp connections

# Disconnect a specific host
tramp disconnect myvm

# Disconnect everything
tramp disconnect --all
```

### Filesystem watching

Watch a remote path for filesystem changes using the RPC agent's
inotify/kqueue integration. Requires the RPC agent to be deployed
(automatic for SSH and standalone Docker/K8s backends).

```nushell
# Watch /app for 5 seconds and collect change events
> tramp watch /ssh:myvm:/app --duration 5000
╭───┬──────────────────────┬────────┬─────────────────────────╮
│ # │        paths         │  kind  │       timestamp         │
├───┼──────────────────────┼────────┼─────────────────────────┤
│ 0 │ [/app/config.toml]   │ modify │ 2025-01-15 10:23:45.123 │
│ 1 │ [/app/data/cache.db] │ create │ 2025-01-15 10:23:46.456 │
╰───┴──────────────────────┴────────┴─────────────────────────╯

# Watch recursively (include subdirectories)
> tramp watch /ssh:myvm:/app --recursive --duration 10000

# List currently active watches on the remote
> tramp watch /ssh:myvm:/ --list
╭───┬──────┬───────────╮
│ # │ path │ recursive │
├───┼──────┼───────────┤
│ 0 │ /app │ true      │
╰───┴──────┴───────────╯

# Stop watching a path
> tramp watch /ssh:myvm:/app --remove
```

### Interactive remote shell

Open a full interactive terminal session on a remote host. This uses the
RPC agent's PTY support to provide a seamless shell-in-shell experience
without leaving Nushell. Requires the RPC agent (automatic for SSH and
standalone Docker/K8s backends). Unix only.

```nushell
# Open a shell on a remote host (auto-detects remote $SHELL)
> tramp shell /ssh:myvm:/

# Open a shell with a specific working directory
> tramp shell /ssh:myvm:/app

# Use a specific shell program
> tramp shell /ssh:myvm:/ --shell /bin/bash

# Shell inside a Docker container
> tramp shell /docker:mycontainer:/

# Shell inside a container on a remote host (chained path)
> tramp shell /ssh:myvm|docker:ctr:/
```

The terminal is put in raw mode for the duration of the session — control
characters like Ctrl-C are forwarded to the remote shell rather than
interpreted locally. The session ends when the remote shell exits (e.g.
type `exit` or press Ctrl-D). Terminal resize events are detected
automatically and forwarded to the remote PTY.

### Cache configuration

The stat and directory listing caches default to a 5-second TTL. Override
this at runtime via the `$env.TRAMP_CACHE_TTL` environment variable:

```nushell
# Set cache TTL to 10 seconds
$env.TRAMP_CACHE_TTL = 10sec

# Use a shorter TTL for rapid iteration
$env.TRAMP_CACHE_TTL = 1sec

# Disable caching entirely (every command hits the remote)
$env.TRAMP_CACHE_TTL = 0sec

# String values also work: "5", "2.5", "500ms", "10s", "3sec"
$env.TRAMP_CACHE_TTL = "500ms"
```

The TTL is read from the environment on each command invocation, so you can
change it on-the-fly without restarting the plugin.

### Tab completion

All `tramp` commands that accept path arguments provide dynamic tab
completion. Just press `Tab` at any point while typing a TRAMP path:

```nushell
# Complete backend prefixes
tramp open /<Tab>
# → /ssh:  /docker:  /k8s:  /sudo:

# Complete host names (from ~/.ssh/config, active connections, docker ps, kubectl get pods)
tramp open /ssh:<Tab>
# → /ssh:myvm:  /ssh:webserver:  /ssh:jumpbox:

tramp open /docker:<Tab>
# → /docker:webapp:  /docker:postgres:

# Start remote path after host
tramp open /ssh:myvm:<Tab>
# → /ssh:myvm:/

# Complete remote directories and files
tramp open /ssh:myvm:/<Tab>
# → /ssh:myvm:/etc/  /ssh:myvm:/home/  /ssh:myvm:/var/  ...

tramp open /ssh:myvm:/etc/ho<Tab>
# → /ssh:myvm:/etc/hostname  /ssh:myvm:/etc/hosts

# Relative path completion works when a remote CWD is set
tramp cd /ssh:myvm:/app
tramp open conf<Tab>
# → config.toml  config.json
```

Both positional arguments of `tramp cp` support completions, so you get
suggestions for source and destination paths independently.

## Path Format

```text
/<backend>:<user>@<host>#<port>:<remote-path>
```

| Segment       | Required | Example             |
|---------------|----------|---------------------|
| `backend`     | ✅       | `ssh`, `docker`, `k8s`, `sudo` |
| `user`        | ❌       | `admin`             |
| `host`        | ✅       | `myvm`, `10.0.0.1`, container/pod name, target user |
| `port`        | ❌       | `2222`              |
| `remote-path` | ✅       | `/etc/config`       |

### Backend reference

| Backend | Host field | User field | Example |
|---------|-----------|------------|---------|
| `ssh` | Hostname or IP | SSH user | `/ssh:admin@myvm#2222:/path` |
| `docker` | Container name/ID | `--user` inside container | `/docker:root@webapp:/path` |
| `k8s` / `kubernetes` | Pod name | `-c` container name | `/k8s:sidecar@mypod:/path` |
| `sudo` | Target user | (unused) | `/sudo:root:/path` |

### Single-hop examples

```text
/ssh:myvm:/etc/config
/ssh:admin@myvm#2222:/etc/config
/docker:mycontainer:/app/config.toml
/k8s:mypod:/tmp/data
/k8s:sidecar@mypod:/app/logs
/sudo:root:/etc/shadow
```

### Chained path examples

```text
/ssh:myvm|docker:webapp:/app/config.toml
/ssh:myvm|sudo:root:/etc/shadow
/ssh:jumpbox|docker:webapp|sudo:root:/etc/shadow
```

> **Note**: SSH-through-SSH chaining (`/ssh:jump|ssh:target:/path`) is not
> supported directly. Use `ProxyJump` in `~/.ssh/config` instead — it's
> more efficient and fully supported by the SSH backend.

## Commands

| Command               | Description                                        |
|-----------------------|----------------------------------------------------|
| `tramp`               | Show help and usage information                    |
| `tramp open`          | Read a remote file and return as Nushell value     |
| `tramp ls`            | List a remote directory as a table (with rich metadata) |
| `tramp save`          | Write piped data to a remote file                  |
| `tramp rm`            | Delete a remote file                               |
| `tramp cp`            | Copy files between local/remote locations          |
| `tramp cd`            | Set the remote working directory                   |
| `tramp pwd`           | Show the current remote working directory          |
| `tramp exec`          | Execute a command on the remote (push execution)   |
| `tramp info`          | Show remote system info (OS, arch, hostname, disk) |
| `tramp ping`          | Test connectivity to a remote host                 |
| `tramp connections`   | List active pooled connections                     |
| `tramp disconnect`    | Close connections (by host or `--all`)             |
| `tramp watch`         | Watch a remote path for filesystem changes (requires RPC agent) |
| `tramp shell`         | Interactive remote terminal session (requires RPC agent, Unix only) |

## Requirements

- **Nushell** ≥ 0.110
- **OpenSSH** client installed and in `$PATH` (`ssh`, `ssh-agent`) — for SSH backend
- **Docker CLI** installed and in `$PATH` — for Docker backend
- **kubectl** installed and configured — for Kubernetes backend
- **sudo** configured for non-interactive use (`NOPASSWD`) — for Sudo backend
- **GNU coreutils** on the remote/target (`stat` for listings; `cat`, `rm`, `base64` as fallback when SFTP is unavailable)

When the `tramp-agent` binary is deployed on the remote host, GNU coreutils are no longer required — all operations use native syscalls.

## Architecture

The project is organised as a Cargo workspace with two crates:

```text
nu-plugin-tramp/
├── crates/
│   ├── plugin/     # nu-plugin-tramp — Nushell plugin
│   └── agent/      # tramp-agent — lightweight RPC agent
├── Cargo.toml      # workspace root
└── ...
```

### Plugin architecture

```text
Nushell command
      │
      ▼
┌─────────────────────────────────────────────┐
│          nu-plugin-tramp plugin             │
│                                             │
│  ┌─────────────┐   ┌───────────────────┐    │
│  │ Path Parser │──▶│ Chain Resolver    │    │
│  └─────────────┘   └───────┬───────────┘    │
│                            │                │
│                   ┌────────▼────────┐       │
│                   │   VFS Layer     │       │
│                   │  ┌───────────┐  │       │
│                   │  │ Stat/List │  │       │
│                   │  │  Cache    │  │       │
│                   │  └───────────┘  │       │
│                   └────────┬────────┘       │
│                            │                │
│         ┌──────────────────┼──────────┐     │
│         ▼         ▼        ▼          ▼     │
│      ┌─────┐  ┌────────┐ ┌─────┐ ┌──────┐  │
│      │ SSH │  │ Docker │ │ K8s │ │ Sudo │  │
│      └─────┘  └────────┘ └─────┘ └──────┘  │
│                   ▲        ▲        ▲       │
│                   └────────┴────────┘       │
│               CommandRunner abstraction     │
│            (LocalRunner / RemoteRunner)      │
└─────────────────────────────────────────────┘
```

### RPC Agent architecture

The `tramp-agent` binary runs on the remote host and replaces shell-command
parsing with native syscalls over a MsgPack-RPC connection. The agent
supports three transport modes:

1. **stdin/stdout** (default) — piped through an SSH session or `docker exec -i`
2. **TCP listener** (`--listen tcp:0.0.0.0:9547`) — direct socket connection
3. **Unix socket** (`--listen unix:/tmp/tramp-agent.sock`) — local IPC

```text
┌──────────────────┐  SSH / TCP / Unix  ┌───────────────────────────┐
│  nu-plugin-tramp │ ◄───────────────► │      tramp-agent          │
│  (local Nushell) │   MsgPack-RPC     │      (remote Rust)        │
└──────────────────┘   4-byte len +    │                           │
                       MessagePack     │  ┌──────┐ ┌─────────┐    │
                                       │  │ file │ │   dir   │    │
                                       │  └──────┘ └─────────┘    │
                                       │  ┌──────┐ ┌─────────┐    │
                                       │  │ proc │ │ system  │    │
                                       │  └──────┘ └─────────┘    │
                                       │  ┌──────┐ ┌─────────┐    │
                                       │  │batch │ │  watch  │    │
                                       │  └──────┘ └─────────┘    │
                                       │  ┌──────┐                │
                                       │  │ pty  │                │
                                       │  └──────┘                │
                                       └───────────────────────────┘
```

| Category   | Methods                                                        |
|------------|----------------------------------------------------------------|
| File       | `file.stat`, `file.stat_batch`, `file.truename`, `file.size`,  |
|            | `file.read`, `file.read_range`, `file.write`,                  |
|            | `file.write_range`, `file.copy`, `file.rename`,               |
|            | `file.delete`, `file.set_modes`                                |
| Directory  | `dir.list`, `dir.create`, `dir.remove`                         |
| Process    | `process.run`, `process.start`, `process.read`,               |
|            | `process.write`, `process.kill`                               |
| PTY        | `process.start_pty`, `process.resize`                          |
| System     | `system.info`, `system.getenv`, `system.statvfs`              |
| Batch      | `batch` (N ops in 1 round-trip, sequential or parallel)        |
| Watch      | `watch.add`, `watch.remove`, `watch.list` → `fs.changed`      |

Key advantages over shell parsing:

| Aspect             | Current (shell parsing)        | With tramp-agent              |
|--------------------|-------------------------------|-------------------------------|
| Communication      | Individual SSH exec commands   | MsgPack-RPC over single pipe  |
| Latency            | N operations = N round-trips   | N operations = 1 round-trip   |
| Binary data        | base64 encode/decode           | Native binary (MsgPack bin)   |
| Stat + list        | Separate commands, text parse  | Native `lstat()` syscalls     |
| Shell dependency   | Requires sh, stat, cat, etc.  | None (self-contained binary)  |
| Cache invalidation | TTL-based (5s default)         | inotify/kqueue push events    |

### Automatic agent deployment (SSH)

When the plugin connects to a remote host via SSH, it attempts to deploy
and start the `tramp-agent` binary automatically:

```text
Connect via SSH
       │
       ▼
Agent already deployed? ──yes──► Start agent, use RPC backend
       │ no
       ▼
Detect remote arch (uname -sm)
       │
       ▼
Check local cache (~/.cache/nu-plugin-tramp/<version>/<triple>/tramp-agent)
       │
       ├─ Found ──────────────► Upload via SFTP, chmod, start
       │
       ▼
No cached binary available
       │
       ▼
Fall back to shell-parsing mode (SshBackend)
```

The deployment is completely transparent — if anything fails at any step,
the plugin silently falls back to the existing shell-parsing SSH backend.
When the agent is available, an `RpcBackend` replaces `SshBackend` and all
operations go through the MsgPack-RPC pipe instead of spawning individual
shell commands.

To pre-cache an agent binary for a target, place it at:
`~/.cache/nu-plugin-tramp/<version>/<target-triple>/tramp-agent`

Supported target triples: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
`x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-freebsd`.

### Container agent deployment

The same agent deployment concept extends to Docker and Kubernetes backends.
For standalone containers (not behind an SSH hop), the plugin attempts to
copy the `tramp-agent` binary into the container and start it as an
interactive process with piped stdin/stdout:

```text
Standalone Docker/K8s:
  ┌──────────────────┐  docker exec -i   ┌───────────────────┐
  │  nu-plugin-tramp  │ ◄──────────────► │   tramp-agent     │
  │  (local Nushell)  │   MsgPack-RPC    │   (in container)  │
  └──────────────────┘                   └───────────────────┘

Chained (SSH → Docker):
  ┌──────────────────┐  SSH pipe  ┌───────────┐ docker exec ┌───────────┐
  │  nu-plugin-tramp  │ ◄───────► │   agent   │ ◄─────────► │ container │
  │  (local Nushell)  │ MsgPack   │  (remote) │  shell cmds  │           │
  └──────────────────┘            └───────────┘              └───────────┘
```

The deployment flow for containers (`crates/plugin/src/backend/deploy_exec.rs`):

1. Detect container arch via `docker exec <ctr> uname -sm` / `kubectl exec <pod> -- uname -sm`
2. Check if agent is already deployed inside the container
3. Find cached binary locally → copy into container:
   - Docker: `docker cp <local_tmp> <container>:/tmp/tramp-agent-dir/tramp-agent`
   - Kubernetes: `kubectl cp` with base64 exec fallback (for minimal images lacking `tar`)
4. Start agent: `docker exec -i <ctr> /tmp/tramp-agent-dir/tramp-agent` (or `kubectl exec -i`)
5. Ping to verify, then switch to `RpcBackend`

For chained paths (e.g. `/ssh:host|docker:ctr:/path`), the parent SSH hop
already uses the RPC agent, so Docker/K8s commands routed through it benefit
from the SSH agent's performance. The shell-parsing `ExecBackend` is used
for the container hop in this case.

### Layers

1. **Path Parser** (`crates/plugin/src/protocol.rs`) — Parses TRAMP URIs into structured types with round-trip fidelity; supports multi-hop chained paths
2. **Backend Trait** (`crates/plugin/src/backend/mod.rs`) — Async trait defining `read`, `write`, `list`, `stat`, `exec`, `delete`, and `check` (health-check)
3. **CommandRunner** (`crates/plugin/src/backend/runner.rs`) — Abstraction for executing commands locally (`LocalRunner`) or through a parent backend (`RemoteRunner`), enabling path chaining
4. **ExecBackend** (`crates/plugin/src/backend/exec.rs`) — Generic exec-based backend used by Docker, Kubernetes, and Sudo; wraps commands with a configurable prefix
5. **SSH Backend** (`crates/plugin/src/backend/ssh.rs`) — Uses SFTP for file read/write/delete (fast-path) with automatic fallback to remote command execution; listing and stat use batch-stat (single remote command for all metadata including owner, group, nlinks, inode, symlink targets)
6. **RPC Backend** (`crates/plugin/src/backend/rpc.rs`) — Implements `Backend` by sending MsgPack-RPC calls to a running `tramp-agent`; used automatically when the agent is deployed
7. **RPC Client** (`crates/plugin/src/backend/rpc_client.rs`) — Client-side MsgPack-RPC framing (length-prefixed messages, request/response matching, notification buffering)
8. **SSH Agent Deployment** (`crates/plugin/src/backend/deploy.rs`) — Detects remote arch, manages local binary cache, uploads agent via SFTP or exec fallback, starts the agent process
9. **Container Agent Deployment** (`crates/plugin/src/backend/deploy_exec.rs`) — Deploys agent into Docker/K8s containers via `docker cp`/`kubectl cp` (with base64 fallback), starts agent via interactive `docker exec -i`/`kubectl exec -i`
10. **VFS** (`crates/plugin/src/vfs.rs`) — Resolves paths to backends, builds multi-hop chains, manages connection pooling with health-checks, provides stat/list caching with TTL, bridges async↔sync; automatically attempts RPC backend for SSH, Docker, and Kubernetes hops
11. **RPC Protocol** (`crates/agent/src/rpc.rs`) — Length-prefixed MsgPack framing with Request/Response/Notification message types
12. **Agent Operations** (`crates/agent/src/ops/`) — Native implementations of file, directory, process, system, batch, and watch operations

### Chaining internals

When resolving a chained path like `/ssh:myvm|docker:ctr:/path`, the VFS:

1. Creates an `SshBackend` for the first hop (`ssh:myvm`)
2. Creates a `RemoteRunner` wrapping the SSH backend
3. Creates an `ExecBackend::docker(remote_runner, "ctr")` for the second hop
4. The Docker backend's commands (e.g. `docker exec ctr cat /path`) execute through the SSH session

This composable design means any combination of backends can be chained (except SSH-through-SSH, which should use ProxyJump).

## Roadmap

### Phase 1 — MVP ✅

- [x] TRAMP URI parser (single-hop SSH only)
- [x] SSH backend via `openssh` crate
- [x] `tramp open`, `tramp ls`, `tramp save`, `tramp rm`
- [x] Nushell plugin compiles and registers correctly
- [x] README with install + usage
- [x] Nix package derivation

### Phase 2 — Daily Driver ✅

- [x] Connection pooling with health-checks + automatic reconnection
- [x] `tramp cd` / `tramp pwd` with relative path resolution
- [x] Stat + directory listing cache (with 5s TTL)
- [x] `tramp cp` between local/remote/remote
- [x] `tramp ping`, `tramp connections`, `tramp disconnect` commands
- [x] SFTP fast-path for file read/write/delete with automatic exec fallback

### Phase 3 — Power Features ✅

- [x] Path chaining (SSH → Docker, SSH → Sudo, triple chains, etc.)
- [x] Docker backend (`docker exec`)
- [x] Kubernetes backend (`kubectl exec`)
- [x] Sudo backend
- [x] Push execution model (`tramp exec`)

### Phase 4 — Polish & Usability ✅

- [x] `tramp info` command — remote system info (OS, arch, hostname, user, disk) in one round-trip
- [x] Richer `tramp ls` metadata — owner, group, nlinks, inode, symlink targets
- [x] Batch stat in listings — all metadata gathered in a single remote command
- [x] Configurable cache TTL via `$env.TRAMP_CACHE_TTL`
- [x] Home Manager module for auto-registration (`hm-module.nix`)
- [x] Glob/wildcard support for `tramp ls --glob` and `tramp cp --glob`
- [x] Tab completion for remote paths

### Phase 5 — RPC Agent ✅ (inspired by [emacs-tramp-rpc](https://github.com/ArthurHeymans/emacs-tramp-rpc))

- [x] `tramp-agent` binary — lightweight Rust RPC server (MsgPack-RPC over stdin/stdout)
- [x] Protocol design — 4-byte length-prefixed MessagePack framing with JSON-RPC 2.0-style messages
- [x] File operations — `file.stat`, `file.stat_batch`, `file.truename`, `file.read`, `file.write`, `file.copy`, `file.rename`, `file.delete`, `file.set_modes`
- [x] Directory operations — `dir.list` (with full lstat metadata), `dir.create`, `dir.remove`
- [x] Process operations — `process.run`, `process.start`, `process.read`, `process.write`, `process.kill`
- [x] System operations — `system.info`, `system.getenv`, `system.statvfs`
- [x] Batch operations — `batch` (N ops in 1 round-trip, sequential or parallel)
- [x] Filesystem watching — `watch.add`, `watch.remove`, `watch.list` via inotify/kqueue with `fs.changed` push notifications
- [x] Cargo workspace restructure — plugin and agent as separate crates
- [x] Automatic agent deployment (detect arch, upload, fallback to shell-parsing)
- [x] Plugin RPC backend (switch SSH backend to use agent when available)
- [x] Agent for exec backends (deploy inside Docker/K8s containers)

### Phase 6 — Future ✅

- [x] Streaming for very large files (chunked RPC reads/writes)
- [x] PTY support via agent (remote terminal emulation)
- [x] `tramp watch` command — subscribe to filesystem change notifications
- [x] Agent version management and auto-upgrade
- [x] TCP/Unix socket transport for agent (local Docker without SSH)
- [x] Cross-compilation matrix for agent binaries (x86_64/aarch64 × Linux/macOS)

## License

MIT