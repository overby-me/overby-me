# Getting Started with Tangled CLI

This guide will help you get up and running with the Tangled CLI.

## Installation

### Prerequisites

- Rust toolchain 1.70 or later
- Git
- A Bluesky/AT Protocol account

### Build from Source

1. Clone the repository:

   ```sh
   git clone https://tangled.org/vitorpy.com/tangled-cli
   cd tangled-cli
   ```

2. Build the project:

   ```sh
   cargo build --release
   ```

3. The binary will be available at `target/release/tangled-cli`. Optionally, add it to your PATH or create an alias:

   ```sh
   alias tangled='./target/release/tangled-cli'
   ```

### Install from AUR (Arch Linux)

If you're on Arch Linux, you can install from the AUR:

```sh
yay -S tangled-cli-git
```

## First Steps

### 1. Authenticate

Login with your AT Protocol credentials (your Bluesky account):

```sh
tangled auth login
```

You'll be prompted for your handle (e.g., `alice.bsky.social`) and password. If you're using a custom PDS, specify it with the `--pds` flag:

```sh
tangled auth login --pds https://your-pds.example.com
```

Your credentials are stored securely in your system keyring.

### 2. Check Your Status

Verify you're logged in:

```sh
tangled auth status
```

### 3. List Your Repositories

See all your repositories:

```sh
tangled repo list
```

Or view someone else's public repositories:

```sh
tangled repo list --user alice.bsky.social
```

### 4. Create a Repository

Create a new repository on Tangled:

```sh
tangled repo create my-project --description "My awesome project"
```

By default, repositories are created on the default knot (`tngl.sh`). You can specify a different knot:

```sh
tangled repo create my-project --knot knot1.tangled.sh
```

### 5. Clone a Repository

Clone a repository to start working on it:

```sh
tangled repo clone alice/my-project
```

This uses SSH by default. For HTTPS:

```sh
tangled repo clone alice/my-project --https
```

## Working with Issues

### Create an Issue

```sh
tangled issue create --repo my-project --title "Add new feature" --body "We should add feature X"
```

### List Issues

```sh
tangled issue list --repo my-project
```

### View Issue Details

```sh
tangled issue show <issue-id>
```

### Comment on an Issue

```sh
tangled issue comment <issue-id> --body "I'm working on this!"
```

## Working with Pull Requests

### Create a Pull Request

```sh
tangled pr create --repo my-project --base main --head feature-branch --title "Add feature X"
```

The CLI will use `git format-patch` to create a patch from your branch.

### List Pull Requests

```sh
tangled pr list --repo my-project
```

### Review a Pull Request

```sh
tangled pr review <pr-id> --approve --comment "Looks good!"
```

Or request changes:

```sh
tangled pr review <pr-id> --request-changes --comment "Please fix the tests"
```

### Merge a Pull Request

```sh
tangled pr merge <pr-id>
```

## CI/CD with Spindle

Spindle is Tangled's integrated CI/CD system.

### Enable Spindle for Your Repository

```sh
tangled spindle config --repo my-project --enable
```

Or use a custom spindle URL:

```sh
tangled spindle config --repo my-project --url https://my-spindle.example.com
```

### View Pipeline Runs

```sh
tangled spindle list --repo my-project
```

### Stream Workflow Logs

```sh
tangled spindle logs knot:rkey:workflow-name
```

Add `--follow` to tail the logs in real-time.

### Manage Secrets

Add secrets for your CI/CD workflows:

```sh
tangled spindle secret add --repo my-project --key API_KEY --value "my-secret-value"
```

List secrets:

```sh
tangled spindle secret list --repo my-project
```

Remove a secret:

```sh
tangled spindle secret remove --repo my-project --key API_KEY
```

## Advanced Topics

### Repository Migration

Move a repository to a different knot:

```sh
tangled knot migrate --repo my-project --to knot2.tangled.sh
```

This command must be run from within the repository's working directory, and your working tree must be clean and pushed.

### Output Formats

Most commands support JSON output:

```sh
tangled repo list --format json
```

### Quiet and Verbose Modes

Reduce output:

```sh
tangled --quiet repo list
```

Increase verbosity for debugging:

```sh
tangled --verbose repo list
```

## Configuration

The CLI stores configuration in:

- Linux: `~/.config/tangled/config.toml`
- macOS: `~/Library/Application Support/tangled/config.toml`
- Windows: `%APPDATA%\tangled\config.toml`

Session credentials are stored securely in your system keyring (GNOME Keyring, KWallet, macOS Keychain, or Windows Credential Manager).

### Environment Variables

- `TANGLED_PDS_BASE` - Override the default PDS (default: `https://bsky.social`)
- `TANGLED_API_BASE` - Override the Tangled API base (default: `https://tngl.sh`)
- `TANGLED_SPINDLE_BASE` - Override the Spindle base (default: `wss://spindle.tangled.sh`)

## Troubleshooting

### Keyring Issues on Linux

If you see keyring errors on Linux, ensure you have a secret service running:

```sh
# For GNOME
systemctl --user enable --now gnome-keyring-daemon

# For KDE
# KWallet should start automatically with Plasma
```

### Authentication Failures

If authentication fails with your custom PDS:

```sh
tangled auth login --pds https://your-pds.example.com
```

Make sure the PDS URL is correct and accessible.

### "Repository not found" Errors

Verify the repository exists and you have access:

```sh
tangled repo info owner/reponame
```

## Getting Help

For command-specific help, use the `--help` flag:

```sh
tangled --help
tangled repo --help
tangled repo create --help
```

## Next Steps

- Explore all available commands with `tangled --help`
- Set up CI/CD workflows with `.tangled.yml` in your repository
- Check out the main README for more examples and advanced usage

Happy collaborating! 🧶
