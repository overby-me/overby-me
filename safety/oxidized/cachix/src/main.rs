#![allow(dead_code)]

mod api;
mod cli;
mod config;
mod daemon;
mod deploy;
mod nix;
mod nixconf;
mod push;

use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;

use cli::*;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up logging
    let filter = if cli.verbose {
        "cachix=debug,info"
    } else {
        "cachix=info,warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_target(false)
        .init();

    // Load config
    let config_path = cli.config.unwrap_or_else(|| {
        config::Config::default_path().unwrap_or_else(|_| PathBuf::from("cachix.json"))
    });
    let mut cfg = config::Config::load(&config_path)?;

    // Override hostname if set
    if cli.hostname != "https://cachix.org" {
        cfg.hostname = cli.hostname.clone();
    }

    match cli.command {
        Command::Authtoken { stdin, token } => {
            cmd_authtoken(&mut cfg, &config_path, stdin, token).await?;
        }
        Command::Config { command } => {
            cmd_config(&mut cfg, &config_path, command)?;
        }
        Command::GenerateKeypair { cache_name } => {
            cmd_generate_keypair(&mut cfg, &config_path, &cache_name).await?;
        }
        Command::Use {
            cache_name,
            mode,
            nixos_folder,
            output_directory,
        } => {
            cmd_use(
                &cfg,
                &cache_name,
                mode,
                &nixos_folder,
                output_directory.as_deref(),
            )
            .await?;
        }
        Command::Remove {
            cache_name,
            mode,
            nixos_folder,
        } => {
            cmd_remove(&cfg, &cache_name, mode, &nixos_folder).await?;
        }
        Command::Push {
            cache_name,
            paths,
            push_opts,
        } => {
            cmd_push(&cfg, &cache_name, paths, push_opts).await?;
        }
        Command::WatchStore {
            cache_name,
            push_opts,
        } => {
            cmd_watch_store(&cfg, &cache_name, push_opts).await?;
        }
        Command::WatchExec {
            cache_name,
            cmd,
            args,
            watch_mode: _watch_mode,
            push_opts,
        } => {
            cmd_watch_exec(&cfg, &cache_name, &cmd, &args, push_opts).await?;
        }
        Command::Import {
            cache_name,
            s3_uri,
            push_opts,
        } => {
            cmd_import(&cfg, &cache_name, &s3_uri, push_opts).await?;
        }
        Command::Pin {
            cache_name,
            pin_name,
            store_path,
            artifact,
            keep_days,
            keep_revisions,
            keep_forever,
        } => {
            cmd_pin(
                &cfg,
                &cache_name,
                &pin_name,
                &store_path,
                artifact,
                keep_days,
                keep_revisions,
                keep_forever,
            )
            .await?;
        }
        Command::Daemon { command } => {
            cmd_daemon(&cfg, command).await?;
        }
        Command::Deploy { command } => {
            cmd_deploy(&cfg, command).await?;
        }
        Command::Doctor { cache, store_path } => {
            cmd_doctor(&cfg, cache.as_deref(), store_path.as_deref()).await?;
        }
    }

    Ok(())
}

// --- Command implementations ---

async fn cmd_authtoken(
    cfg: &mut config::Config,
    config_path: &Path,
    stdin: bool,
    token: Option<String>,
) -> Result<()> {
    let token = if stdin {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .context("failed to read token from stdin")?;
        line.trim().to_string()
    } else {
        token.context("auth token argument is required (or use --stdin)")?
    };

    if token.is_empty() {
        bail!("auth token cannot be empty");
    }

    cfg.set_auth_token(token);
    cfg.save(config_path)?;
    println!("auth token saved to {}", config_path.display());
    Ok(())
}

fn cmd_config(cfg: &mut config::Config, config_path: &Path, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Get { key } => match key.as_str() {
            "hostname" => println!("{}", cfg.hostname),
            other => bail!("unknown config key: {other}"),
        },
        ConfigCommand::Set { key, value } => {
            match key.as_str() {
                "hostname" => cfg.hostname = value,
                other => bail!("unknown config key: {other}"),
            }
            cfg.save(config_path)?;
            println!("config updated");
        }
    }
    Ok(())
}

async fn cmd_generate_keypair(
    cfg: &mut config::Config,
    config_path: &Path,
    cache_name: &str,
) -> Result<()> {
    let api = make_api_client(cfg)?;

    let (secret_key, public_key) = nix::sign::generate_keypair(cache_name);

    // Upload public key
    api.upload_signing_key(cache_name, &public_key).await?;
    println!("public key uploaded to {cache_name}");

    // Save secret key to config
    cfg.set_secret_key(cache_name, &secret_key);
    cfg.save(config_path)?;
    println!("secret key saved to {}", config_path.display());

    Ok(())
}

async fn cmd_use(
    cfg: &config::Config,
    cache_name: &str,
    mode: InstallModeArg,
    nixos_folder: &Path,
    output_dir: Option<&std::path::Path>,
) -> Result<()> {
    let api = make_api_client(cfg)?;
    let cache = api.get_cache(cache_name).await?;

    let install_mode = match mode {
        InstallModeArg::Nixos => nixconf::InstallMode::NixOS,
        InstallModeArg::RootNixconf => nixconf::InstallMode::RootNixConf,
        InstallModeArg::UserNixconf => nixconf::InstallMode::UserNixConf,
    };

    nixconf::use_cache(
        cache_name,
        &cache.uri,
        &cache.public_signing_keys,
        install_mode,
        nixos_folder,
        output_dir,
    )?;

    // Write netrc for private caches
    if !cache.is_public {
        if let Some(token) = cfg.auth_token.as_deref() {
            let netrc_path = nixconf::write_netrc(&cfg.hostname, token, output_dir)?;
            println!("netrc written to {}", netrc_path.display());
        } else {
            tracing::warn!(
                "cache is private but no auth token is set - you may need to configure netrc manually"
            );
        }
    }

    println!("configured cache '{cache_name}'");
    Ok(())
}

async fn cmd_remove(
    cfg: &config::Config,
    cache_name: &str,
    mode: InstallModeArg,
    nixos_folder: &Path,
) -> Result<()> {
    let api = make_api_client(cfg)?;
    let cache = api.get_cache(cache_name).await?;

    let install_mode = match mode {
        InstallModeArg::Nixos => nixconf::InstallMode::NixOS,
        InstallModeArg::RootNixconf => nixconf::InstallMode::RootNixConf,
        InstallModeArg::UserNixconf => nixconf::InstallMode::UserNixConf,
    };

    nixconf::remove_cache(
        cache_name,
        &cache.uri,
        &cache.public_signing_keys,
        install_mode,
        nixos_folder,
    )?;

    println!("removed cache '{cache_name}'");
    Ok(())
}

async fn cmd_push(
    cfg: &config::Config,
    cache_name: &str,
    paths: Vec<String>,
    push_args: PushArgs,
) -> Result<()> {
    let credential = config::resolve_push_credential(cfg, cache_name)?;
    let api = make_api_client_with_credential(cfg, &credential)?;
    let opts = make_push_options(&push_args);

    // Read paths from stdin if none given
    let paths = if paths.is_empty() {
        let stdin = std::io::stdin();
        stdin
            .lock()
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
            .collect()
    } else {
        paths
    };

    if paths.is_empty() {
        bail!("no store paths provided");
    }

    push::push_paths(&api, cache_name, &paths, &credential, &opts).await?;
    println!("push complete");
    Ok(())
}

async fn cmd_watch_store(
    cfg: &config::Config,
    cache_name: &str,
    push_args: PushArgs,
) -> Result<()> {
    let credential = config::resolve_push_credential(cfg, cache_name)?;
    let api = make_api_client_with_credential(cfg, &credential)?;
    let opts = make_push_options(&push_args);

    println!("watching /nix/store for new paths...");
    let mut rx = nix::store::watch_store()?;

    while let Some(path) = rx.recv().await {
        // Only push valid-looking store paths
        if !path.starts_with("/nix/store/") || path.contains(".drv") || path.contains(".lock") {
            continue;
        }

        // Wait briefly for the path to be fully written
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        if let Ok(true) = nix::store::is_valid_path(&path).await {
            tracing::info!("new store path: {path}");
            if let Err(e) = push::push_store_path(&api, cache_name, &path, &credential, &opts).await
            {
                tracing::error!("failed to push {path}: {e:#}");
            }
        }
    }

    Ok(())
}

async fn cmd_watch_exec(
    cfg: &config::Config,
    cache_name: &str,
    cmd: &str,
    args: &[String],
    push_args: PushArgs,
) -> Result<()> {
    let credential = config::resolve_push_credential(cfg, cache_name)?;
    let api = make_api_client_with_credential(cfg, &credential)?;
    let opts = make_push_options(&push_args);

    // Snapshot store paths before execution
    let before = snapshot_store().await?;

    // Run the command
    tracing::info!("running: {cmd} {}", args.join(" "));
    let status = tokio::process::Command::new(cmd)
        .args(args)
        .status()
        .await
        .with_context(|| format!("failed to execute {cmd}"))?;

    if !status.success() {
        tracing::warn!("command exited with status {status}");
    }

    // Snapshot store paths after execution
    let after = snapshot_store().await?;

    // Find new paths
    let new_paths: Vec<String> = after.difference(&before).cloned().collect();

    if new_paths.is_empty() {
        println!("no new store paths to push");
        return Ok(());
    }

    println!("found {} new store paths", new_paths.len());
    push::push_paths(&api, cache_name, &new_paths, &credential, &opts).await?;
    println!("push complete");
    Ok(())
}

async fn snapshot_store() -> Result<HashSet<String>> {
    let output = tokio::process::Command::new("ls")
        .arg("/nix/store")
        .output()
        .await
        .context("failed to list /nix/store")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().map(|l| format!("/nix/store/{l}")).collect())
}

async fn cmd_import(
    cfg: &config::Config,
    cache_name: &str,
    s3_uri: &str,
    push_args: PushArgs,
) -> Result<()> {
    // Parse S3 URI: s3://bucket?endpoint=...&region=...
    let uri = url::Url::parse(s3_uri).context("invalid S3 URI")?;
    if uri.scheme() != "s3" {
        bail!("expected s3:// URI scheme");
    }

    let bucket = uri
        .host_str()
        .context("missing bucket name in S3 URI")?
        .to_string();
    let params: std::collections::HashMap<String, String> = uri
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let endpoint = params
        .get("endpoint")
        .context("missing endpoint parameter")?
        .clone();
    let region = params
        .get("region")
        .cloned()
        .unwrap_or_else(|| "us-east-1".to_string());

    let credential = config::resolve_push_credential(cfg, cache_name)?;
    let api = make_api_client_with_credential(cfg, &credential)?;
    let opts = make_push_options(&push_args);

    tracing::info!("importing from s3://{bucket} (endpoint: {endpoint}, region: {region})");

    // Use AWS CLI to list NARs from S3
    let output = tokio::process::Command::new("aws")
        .arg("s3")
        .arg("ls")
        .arg(format!("s3://{bucket}/"))
        .arg("--endpoint-url")
        .arg(&endpoint)
        .arg("--region")
        .arg(&region)
        .arg("--recursive")
        .output()
        .await
        .context("failed to list S3 bucket (is aws CLI installed?)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("aws s3 ls failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let nar_files: Vec<&str> = stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.last().copied()
        })
        .filter(|f| f.ends_with(".narinfo"))
        .collect();

    tracing::info!("found {} narinfo files in S3", nar_files.len());

    for narinfo_file in nar_files {
        let hash = narinfo_file.trim_end_matches(".narinfo");

        // Check if already in cache
        if api.head_narinfo(cache_name, hash).await? {
            tracing::debug!("skipping {hash} (already in cache)");
            continue;
        }

        // Download the narinfo from S3
        let narinfo_output = tokio::process::Command::new("aws")
            .arg("s3")
            .arg("cp")
            .arg(format!("s3://{bucket}/{narinfo_file}"))
            .arg("-")
            .arg("--endpoint-url")
            .arg(&endpoint)
            .arg("--region")
            .arg(&region)
            .output()
            .await?;

        if !narinfo_output.status.success() {
            tracing::warn!("failed to download {narinfo_file}");
            continue;
        }

        let narinfo_text = String::from_utf8_lossy(&narinfo_output.stdout);

        // Parse store path from narinfo
        let store_path = narinfo_text
            .lines()
            .find_map(|l| l.strip_prefix("StorePath: "))
            .map(|s| s.trim().to_string());

        if let Some(store_path) = store_path {
            let nar_url = narinfo_text
                .lines()
                .find_map(|l| l.strip_prefix("URL: "))
                .map(|s| s.trim().to_string());

            if let Some(nar_url) = nar_url {
                tracing::info!("importing {store_path} from {nar_url}");

                // Download NAR from S3 to temp, then import via nix-store
                let temp_dir = tempfile::tempdir()?;
                let nar_path = temp_dir.path().join("import.nar");

                let dl_status = tokio::process::Command::new("aws")
                    .arg("s3")
                    .arg("cp")
                    .arg(format!("s3://{bucket}/{nar_url}"))
                    .arg(&nar_path)
                    .arg("--endpoint-url")
                    .arg(&endpoint)
                    .arg("--region")
                    .arg(&region)
                    .status()
                    .await?;

                if !dl_status.success() {
                    tracing::warn!("failed to download NAR for {store_path}");
                    continue;
                }

                // Import into local store
                let import_status = tokio::process::Command::new("nix-store")
                    .arg("--restore")
                    .arg(&store_path)
                    .stdin(std::process::Stdio::from(std::fs::File::open(&nar_path)?))
                    .status()
                    .await?;

                if import_status.success() {
                    if let Err(e) =
                        push::push_store_path(&api, cache_name, &store_path, &credential, &opts)
                            .await
                    {
                        tracing::error!("failed to push {store_path}: {e:#}");
                    }
                } else {
                    tracing::warn!("failed to import {store_path} into local store");
                }
            }
        }
    }

    println!("import complete");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_pin(
    cfg: &config::Config,
    cache_name: &str,
    pin_name: &str,
    store_path: &str,
    artifacts: Vec<String>,
    keep_days: Option<u64>,
    keep_revisions: Option<u64>,
    keep_forever: bool,
) -> Result<()> {
    let api = make_api_client(cfg)?;

    let keep = if keep_forever {
        Some(api::types::PinKeep::Forever)
    } else if let Some(days) = keep_days {
        Some(api::types::PinKeep::Days(days))
    } else {
        keep_revisions.map(api::types::PinKeep::Revisions)
    };

    let pin = api::types::PinCreate {
        name: pin_name.to_string(),
        store_path: store_path.to_string(),
        artifacts,
        keep,
    };

    api.create_pin(cache_name, &pin).await?;
    println!("pin '{pin_name}' created for {store_path}");
    Ok(())
}

async fn cmd_daemon(cfg: &config::Config, command: DaemonCommand) -> Result<()> {
    match command {
        DaemonCommand::Run {
            cache_name,
            daemon_opts,
            push_opts,
        } => {
            let credential = config::resolve_push_credential(cfg, &cache_name)?;
            let api = make_api_client_with_credential(cfg, &credential)?;
            let push_options = make_push_options(&push_opts);

            let daemon_options = daemon::DaemonOptions {
                socket_path: daemon_opts.socket,
                allow_remote_stop: daemon_opts.remote_stop,
                keep_alive_interval_secs: daemon_opts.keep_alive_interval,
                keep_alive_timeout_secs: daemon_opts.keep_alive_timeout,
                narinfo_batch_size: daemon_opts.narinfo_batch_size,
                narinfo_batch_timeout_secs: daemon_opts.narinfo_batch_timeout,
                narinfo_cache_ttl_secs: daemon_opts.narinfo_cache_ttl,
                narinfo_max_cache_size: daemon_opts.narinfo_max_cache_size,
            };

            daemon::run_daemon(api, cache_name, credential, push_options, daemon_options).await?;
        }
        DaemonCommand::Push {
            wait,
            paths,
            daemon_opts,
        } => {
            let paths = if paths.is_empty() {
                let stdin = std::io::stdin();
                stdin
                    .lock()
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| !l.trim().is_empty())
                    .collect()
            } else {
                paths
            };

            daemon::daemon_push(&daemon_opts.socket, paths, wait).await?;
        }
        DaemonCommand::Stop { daemon_opts } => {
            daemon::daemon_stop(&daemon_opts.socket).await?;
        }
        DaemonCommand::WatchExec {
            cache_name: _,
            cmd,
            args,
            daemon_opts,
        } => {
            let before = snapshot_store().await?;

            let status = tokio::process::Command::new(&cmd)
                .args(&args)
                .status()
                .await?;

            if !status.success() {
                tracing::warn!("command exited with status {status}");
            }

            let after = snapshot_store().await?;
            let new_paths: Vec<String> = after.difference(&before).cloned().collect();

            if !new_paths.is_empty() {
                daemon::daemon_push(&daemon_opts.socket, new_paths, true).await?;
            } else {
                println!("no new store paths to push");
            }
        }
        DaemonCommand::Doctor { daemon_opts } => {
            daemon::daemon_doctor(&daemon_opts.socket).await?;
        }
    }
    Ok(())
}

async fn cmd_deploy(cfg: &config::Config, command: DeployCommand) -> Result<()> {
    match command {
        DeployCommand::Activate {
            deploy_spec,
            agent,
            r#async,
        } => {
            let token = std::env::var("CACHIX_ACTIVATE_TOKEN")
                .context("CACHIX_ACTIVATE_TOKEN environment variable is required")?;
            let api = api::ApiClient::new(&cfg.hostname, Some(&token))?;
            deploy::activate(&api, &deploy_spec, &agent, r#async).await?;
        }
        DeployCommand::Agent {
            agent_name,
            profile,
            bootstrap,
        } => {
            let token = std::env::var("CACHIX_AGENT_TOKEN")
                .context("CACHIX_AGENT_TOKEN environment variable is required")?;
            let api = api::ApiClient::new(&cfg.hostname, Some(&token))?;
            deploy::run_agent(&api, &agent_name, &token, Some(&profile), bootstrap).await?;
        }
    }
    Ok(())
}

async fn cmd_doctor(
    cfg: &config::Config,
    cache_name: Option<&str>,
    store_path: Option<&str>,
) -> Result<()> {
    println!("Cachix Doctor");
    println!("=============");

    // Check nix installation
    let nix_version = tokio::process::Command::new("nix")
        .arg("--version")
        .output()
        .await;
    match nix_version {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            println!("Nix: {}", version.trim());
        }
        _ => {
            println!("Nix: NOT FOUND");
            bail!("nix is not installed or not in PATH");
        }
    }

    // Check config
    println!("Config hostname: {}", cfg.hostname);
    println!(
        "Auth token: {}",
        if cfg.auth_token.is_some() {
            "configured"
        } else {
            "not set"
        }
    );

    // Check API connectivity
    let api = make_api_client(cfg)?;
    if let Some(name) = cache_name {
        match api.get_cache(name).await {
            Ok(cache) => {
                println!("Cache '{}': OK (public: {})", name, cache.is_public);
                println!("  URI: {}", cache.uri);
                println!("  Permission: {:?}", cache.permission);
                println!("  Compression: {:?}", cache.preferred_compression_method);

                if let Some(sp) = store_path {
                    let hash = nix::sign::store_path_hash(sp)?;
                    let exists = api.head_narinfo(name, &hash).await?;
                    println!(
                        "  Store path {sp}: {}",
                        if exists { "CACHED" } else { "NOT CACHED" }
                    );
                }
            }
            Err(e) => {
                println!("Cache '{}': FAILED ({})", name, e);
            }
        }
    } else {
        println!("  (no cache specified, use --cache to check a specific cache)");
    }

    // Check nix.conf
    let user_conf = dirs::config_dir().map(|d| d.join("platform/nix/nix.conf"));
    let root_conf = std::path::PathBuf::from("/etc/nix/nix.conf");

    for conf_path in [user_conf, Some(root_conf)].into_iter().flatten() {
        if conf_path.exists() {
            let conf = nixconf::NixConf::load(&conf_path)?;
            println!("nix.conf ({}): found", conf_path.display());
            if let Some(subs) = conf.get("substituters") {
                println!("  substituters: {subs}");
            }
        }
    }

    println!("\nDoctor check complete.");
    Ok(())
}

// --- Helpers ---

fn make_api_client(cfg: &config::Config) -> Result<api::ApiClient> {
    api::ApiClient::new(&cfg.hostname, cfg.auth_token.as_deref())
}

fn make_api_client_with_credential(
    cfg: &config::Config,
    credential: &config::PushCredential,
) -> Result<api::ApiClient> {
    let token = credential.auth_token().or(cfg.auth_token.as_deref());
    api::ApiClient::new(&cfg.hostname, token)
}

fn make_push_options(args: &PushArgs) -> push::PushOptions {
    let compression_method = match args.compression_method {
        Some(CompressionMethodArg::Xz) => api::types::CompressionMethod::Xz,
        Some(CompressionMethodArg::Zstd) => api::types::CompressionMethod::Zstd,
        None => api::types::CompressionMethod::Zstd,
    };

    push::PushOptions {
        compression_method,
        compression_level: args.compression_level,
        chunk_size: args.chunk_size,
        num_concurrent_chunks: args.num_concurrent_chunks,
        jobs: args.jobs,
        omit_deriver: args.omit_deriver,
    }
}
