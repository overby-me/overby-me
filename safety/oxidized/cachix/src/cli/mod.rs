use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Cachix - Nix binary cache hosting CLI
#[derive(Parser, Debug)]
#[command(name = "cachix", version, about)]
pub struct Cli {
    /// Path to the config file
    #[arg(short, long, env = "CACHIX_CONFIG")]
    pub config: Option<PathBuf>,

    /// API hostname
    #[arg(long, env = "CACHIX_HOST", default_value = "https://cachix.org")]
    pub hostname: String,

    /// Enable verbose output
    #[arg(short, long)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Save the auth token to config
    Authtoken {
        /// Read token from stdin
        #[arg(long)]
        stdin: bool,

        /// The auth token
        token: Option<String>,
    },

    /// Manage configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    /// Generate a signing keypair for a cache
    GenerateKeypair {
        /// Name of the binary cache
        cache_name: String,
    },

    /// Configure a binary cache in nix.conf
    Use {
        /// Name of the binary cache
        cache_name: String,

        /// Installation mode
        #[arg(short, long, default_value = "user-nixconf")]
        mode: InstallModeArg,

        /// NixOS configuration folder
        #[arg(short = 'd', long, default_value = "/etc/nixos")]
        nixos_folder: PathBuf,

        /// Output directory for nix.conf and netrc
        #[arg(short = 'O', long)]
        output_directory: Option<PathBuf>,
    },

    /// Remove a binary cache from nix.conf
    Remove {
        /// Name of the binary cache
        cache_name: String,

        /// Installation mode
        #[arg(short, long, default_value = "user-nixconf")]
        mode: InstallModeArg,

        /// NixOS configuration folder
        #[arg(short = 'd', long, default_value = "/etc/nixos")]
        nixos_folder: PathBuf,
    },

    /// Push store paths to a binary cache
    Push {
        /// Name of the binary cache
        cache_name: String,

        /// Store paths to push (reads from stdin if empty)
        paths: Vec<String>,

        #[command(flatten)]
        push_opts: PushArgs,
    },

    /// Watch the Nix store for new paths and push them
    WatchStore {
        /// Name of the binary cache
        cache_name: String,

        #[command(flatten)]
        push_opts: PushArgs,
    },

    /// Run a command and push any built store paths
    WatchExec {
        /// Name of the binary cache
        cache_name: String,

        /// Command to run
        cmd: String,

        /// Arguments for the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,

        /// Watch mode
        #[arg(long, default_value = "auto")]
        watch_mode: WatchModeArg,

        #[command(flatten)]
        push_opts: PushArgs,
    },

    /// Import NARs from S3-compatible storage
    Import {
        /// Name of the binary cache
        cache_name: String,

        /// S3 URI (e.g., s3://bucket?endpoint=https://...)
        s3_uri: String,

        #[command(flatten)]
        push_opts: PushArgs,
    },

    /// Pin a store path in a binary cache
    Pin {
        /// Name of the binary cache
        cache_name: String,

        /// Pin name
        pin_name: String,

        /// Store path to pin
        store_path: String,

        /// Artifact paths
        #[arg(short, long)]
        artifact: Vec<String>,

        /// Keep for N days
        #[arg(long)]
        keep_days: Option<u64>,

        /// Keep N revisions
        #[arg(long)]
        keep_revisions: Option<u64>,

        /// Keep forever
        #[arg(long)]
        keep_forever: bool,
    },

    /// Manage the push daemon
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },

    /// Deploy commands
    Deploy {
        #[command(subcommand)]
        command: DeployCommand,
    },

    /// Check Cachix configuration and connectivity
    Doctor {
        /// Specific cache to check
        #[arg(long)]
        cache: Option<String>,

        /// Store path to check
        store_path: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Get a configuration value
    Get {
        /// The key to get (e.g., "hostname")
        key: String,
    },
    /// Set a configuration value
    Set {
        /// The key to set (e.g., "hostname")
        key: String,
        /// The value to set
        value: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Run the push daemon
    Run {
        /// Name of the binary cache
        cache_name: String,

        #[command(flatten)]
        daemon_opts: DaemonArgs,

        #[command(flatten)]
        push_opts: PushArgs,
    },

    /// Send a push request to the running daemon
    Push {
        /// Wait for push completion
        #[arg(long)]
        wait: bool,

        /// Store paths to push (reads from stdin if empty)
        paths: Vec<String>,

        #[command(flatten)]
        daemon_opts: DaemonArgs,
    },

    /// Stop the running daemon
    Stop {
        #[command(flatten)]
        daemon_opts: DaemonArgs,
    },

    /// Run a command and push built paths through the daemon
    WatchExec {
        /// Name of the binary cache
        cache_name: String,

        /// Command to run
        cmd: String,

        /// Arguments for the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,

        #[command(flatten)]
        daemon_opts: DaemonArgs,
    },

    /// Check daemon health
    Doctor {
        #[command(flatten)]
        daemon_opts: DaemonArgs,
    },
}

#[derive(Subcommand, Debug)]
pub enum DeployCommand {
    /// Activate a deployment
    Activate {
        /// Path to the deploy spec JSON file
        deploy_spec: PathBuf,

        /// Deploy only to specific agents
        #[arg(short, long)]
        agent: Vec<String>,

        /// Don't wait for deployment completion
        #[arg(long)]
        r#async: bool,
    },

    /// Run a deploy agent
    Agent {
        /// Agent name
        agent_name: String,

        /// Nix profile path
        #[arg(default_value = "/nix/var/nix/profiles/system")]
        profile: String,

        /// Exit once system agent takes over
        #[arg(long)]
        bootstrap: bool,
    },
}

/// Shared push options.
#[derive(Parser, Debug, Clone)]
pub struct PushArgs {
    /// Compression level (0-16)
    #[arg(short = 'l', long, default_value = "2")]
    pub compression_level: u32,

    /// Compression method
    #[arg(short = 'm', long)]
    pub compression_method: Option<CompressionMethodArg>,

    /// Multipart upload chunk size in bytes
    #[arg(short = 's', long, default_value = "33554432")]
    pub chunk_size: usize,

    /// Number of concurrent chunk uploads
    #[arg(short = 'n', long, default_value = "4")]
    pub num_concurrent_chunks: usize,

    /// Number of concurrent store path push jobs
    #[arg(short, long, default_value = "8")]
    pub jobs: usize,

    /// Don't publish which derivation built the path
    #[arg(long)]
    pub omit_deriver: bool,
}

/// Shared daemon options.
#[derive(Parser, Debug, Clone)]
pub struct DaemonArgs {
    /// Unix socket path
    #[arg(
        short,
        long,
        env = "CACHIX_DAEMON_SOCKET",
        default_value = "/tmp/cachix-daemon.sock"
    )]
    pub socket: PathBuf,

    /// Allow remote stop
    #[arg(long, default_value = "true")]
    pub remote_stop: bool,

    /// Keep-alive ping interval in seconds
    #[arg(long, default_value = "30")]
    pub keep_alive_interval: u64,

    /// Keep-alive timeout in seconds
    #[arg(long, default_value = "180")]
    pub keep_alive_timeout: u64,

    /// Max paths per narinfo batch
    #[arg(long, default_value = "100")]
    pub narinfo_batch_size: usize,

    /// Max batch wait time in seconds
    #[arg(long, default_value = "0.5")]
    pub narinfo_batch_timeout: f64,

    /// Narinfo cache TTL in seconds
    #[arg(long, default_value = "300")]
    pub narinfo_cache_ttl: u64,

    /// Max narinfo cache entries (0 = unlimited)
    #[arg(long, default_value = "0")]
    pub narinfo_max_cache_size: usize,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum InstallModeArg {
    Nixos,
    RootNixconf,
    UserNixconf,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum CompressionMethodArg {
    Xz,
    Zstd,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum WatchModeArg {
    Auto,
    Store,
    PostBuildHook,
}
