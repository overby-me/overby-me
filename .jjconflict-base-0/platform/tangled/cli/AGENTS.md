# Tangled CLI – Current Implementation Status

This document provides an overview of the Tangled CLI implementation status for AI agents or developers working on the project.

## Implementation Status

### ✅ Fully Implemented

#### Authentication (`auth`)

- `login` - Authenticate with AT Protocol using `com.atproto.server.createSession`
- `status` - Show current authentication status
- `logout` - Clear stored session from keyring

#### Repositories (`repo`)

- `list` - List repositories using `com.atproto.repo.listRecords` with `collection=sh.tangled.repo`
- `create` - Create repositories with two-step flow:
  1. Create PDS record via `com.atproto.repo.createRecord`
  2. Initialize bare repo via `sh.tangled.repo.create` with ServiceAuth
- `clone` - Clone repositories using libgit2 with SSH agent support
- `info` - Display repository information including stats and languages
- `delete` - Delete repositories (both PDS record and knot repo)
- `star` / `unstar` - Star/unstar repositories via `sh.tangled.feed.star`

#### Issues (`issue`)

- `list` - List issues via `com.atproto.repo.listRecords` with `collection=sh.tangled.repo.issue`
- `create` - Create issues via `com.atproto.repo.createRecord`
- `show` - Show issue details and comments
- `edit` - Edit issue title, body, or state
- `comment` - Add comments to issues

#### Pull Requests (`pr`)

- `list` - List PRs via `com.atproto.repo.listRecords` with `collection=sh.tangled.repo.pull`
- `create` - Create PRs using `git format-patch` for patches
- `show` - Show PR details and diff
- `review` - Review PRs with approve/request-changes flags
- `merge` - Merge PRs via `sh.tangled.repo.merge` with ServiceAuth

#### Knot Management (`knot`)

- `migrate` - Migrate repositories between knots
  - Validates working tree is clean and pushed
  - Creates new repo on target knot with source seeding
  - Updates PDS record to point to new knot

#### Spindle CI/CD (`spindle`)

- `config` - Enable/disable or configure spindle URL for a repository
  - Updates the `spindle` field in `sh.tangled.repo` record
- `list` - List pipeline runs via `com.atproto.repo.listRecords` with `collection=sh.tangled.pipeline`
- `logs` - Stream workflow logs via WebSocket (`wss://spindle.tangled.sh/spindle/logs/{knot}/{rkey}/{name}`)
- `secret list` - List secrets via `sh.tangled.repo.listSecrets` with ServiceAuth
- `secret add` - Add secrets via `sh.tangled.repo.addSecret` with ServiceAuth
- `secret remove` - Remove secrets via `sh.tangled.repo.removeSecret` with ServiceAuth

### 🚧 Partially Implemented / Stubs

#### Spindle CI/CD (`spindle`)

- `run` - Manually trigger a workflow (stub)
  - **TODO**: Parse `.tangled.yml` to determine workflows
  - **TODO**: Create pipeline record and trigger spindle ingestion
  - **TODO**: Support manual trigger inputs

## Architecture Overview

### Workspace Structure

- `crates/tangled-cli` - CLI binary with clap-based argument parsing
- `crates/tangled-config` - Configuration and keyring-backed session management
- `crates/tangled-api` - XRPC client wrapper for AT Protocol and Tangled APIs
- `crates/tangled-git` - Git operation helpers (currently unused)

### Key Patterns

#### ServiceAuth Flow

Many Tangled API operations require ServiceAuth tokens:

1. Obtain token via `com.atproto.server.getServiceAuth` from PDS
   - `aud` parameter must be `did:web:<target-host>`
   - `exp` parameter should be Unix timestamp + 600 seconds
2. Use token as `Authorization: Bearer <serviceAuth>` for Tangled API calls

#### Repository Creation Flow

Two-step process:

1. **PDS**: Create `sh.tangled.repo` record via `com.atproto.repo.createRecord`
2. **Tangled API**: Initialize bare repo via `sh.tangled.repo.create` with ServiceAuth

#### Repository Listing

Done entirely via PDS (not Tangled API):

1. Resolve handle → DID if needed via `com.atproto.identity.resolveHandle`
2. List records via `com.atproto.repo.listRecords` with `collection=sh.tangled.repo`
3. Filter client-side (e.g., by knot)

#### Pull Request Merging

1. Fetch PR record to get patch and target branch
2. Obtain ServiceAuth token
3. Call `sh.tangled.repo.merge` with `{did, name, patch, branch, commitMessage, commitBody}`

### Base URLs and Defaults

- **PDS Base** (auth + record operations): Default `https://bsky.social`, stored in session
- **Tangled API Base** (server operations): Default `https://tngl.sh`, can override via `TANGLED_API_BASE`
- **Spindle Base** (CI/CD): Default `wss://spindle.tangled.sh` for WebSocket logs, can override via `TANGLED_SPINDLE_BASE`

### Session Management

Sessions are stored in the system keyring:

- Linux: GNOME Keyring / KWallet via Secret Service API
- macOS: macOS Keychain
- Windows: Windows Credential Manager

Session includes:

```rust
struct Session {
    access_jwt: String,
    refresh_jwt: String,
    did: String,
    handle: String,
    pds: Option<String>, // PDS base URL
}
```

## Working with tangled-core

The `../tangled-core` repository contains the server implementation and lexicon definitions.

### Key Files to Check

- **Lexicons**: `../tangled-core/lexicons/**/*.json`
  - Defines XRPC method schemas (NSIDs, parameters, responses)
  - Example: `sh.tangled.repo.create`, `sh.tangled.repo.merge`

- **XRPC Routes**: `../tangled-core/knotserver/xrpc/xrpc.go`
  - Shows which endpoints require ServiceAuth
  - Maps NSIDs to handler functions

- **API Handlers**: `../tangled-core/knotserver/xrpc/*.go`
  - Implementation details for server-side operations
  - Example: `create_repo.go`, `merge.go`

### Useful Search Commands

```bash
# Find a specific NSID
rg -n "sh\.tangled\.repo\.create" ../tangled-core

# List all lexicons
ls ../tangled-core/lexicons/repo

# Check ServiceAuth usage
rg -n "ServiceAuth|VerifyServiceAuth" ../tangled-core
```

## Next Steps for Contributors

### Priority: Implement `spindle run`

The only remaining stub is `spindle run` for manually triggering workflows. Implementation plan:

1. **Parse `.tangled.yml`** in the current repository to extract workflow definitions
   - Look for workflow names, triggers, and manual trigger inputs

2. **Create pipeline record** on PDS via `com.atproto.repo.createRecord`:

   ```rust
   collection: "sh.tangled.pipeline"
   record: {
       triggerMetadata: {
           kind: "manual",
           repo: { knot, did, repo, defaultBranch },
           manual: { inputs: [...] }
       },
       workflows: [{ name, engine, clone, raw }]
   }
   ```

3. **Notify spindle** (if needed) or let the ingester pick up the new record

4. **Support workflow selection** when multiple workflows exist:
   - `--workflow <name>` flag to select specific workflow
   - Default to first workflow if not specified

5. **Support manual inputs** (if workflow defines them):
   - Prompt for input values or accept via flags

### Code Quality Tasks

- Add more comprehensive error messages for common failure cases
- Improve table formatting for list commands (consider using `tabled` crate features)
- Add shell completion generation (bash, zsh, fish)
- Add more unit tests with `mockito` for API client methods
- Add integration tests with `assert_cmd` for CLI commands

### Documentation Tasks

- Add man pages for all commands
- Create video tutorials for common workflows
- Add troubleshooting guide for common issues

## Development Workflow

### Building

```sh
cargo build              # Debug build
cargo build --release    # Release build
```

### Running

```sh
cargo run -p tangled-cli -- <command>
```

### Testing

```sh
cargo test               # Run all tests
cargo test -- --nocapture  # Show println output
```

### Code Quality

```sh
cargo fmt                # Format code
cargo clippy             # Run linter
cargo clippy -- -W clippy::all  # Strict linting
```

## Troubleshooting Common Issues

### Keyring Errors on Linux

Ensure a secret service is running:

```sh
systemctl --user enable --now gnome-keyring-daemon
```

### Invalid Token Errors

- For record operations: Use PDS client, not Tangled API client
- For server operations: Ensure ServiceAuth audience DID matches target host

### Repository Not Found

- Verify repo exists: `tangled repo info owner/name`
- Check you're using the correct owner (handle or DID)
- Ensure you have access permissions

### WebSocket Connection Failures

- Check spindle base URL is correct (default: `wss://spindle.tangled.sh`)
- Verify the job_id format: `knot:rkey:name`
- Ensure the workflow has actually run and has logs

## Additional Resources

- Main README: `README.md` - User-facing documentation
- Getting Started Guide: `docs/getting-started.md` - Tutorial for new users
- Lexicons: `../tangled-core/lexicons/` - XRPC method definitions
- Server Implementation: `../tangled-core/knotserver/` - Server-side code

---

Last updated: 2025-10-14
