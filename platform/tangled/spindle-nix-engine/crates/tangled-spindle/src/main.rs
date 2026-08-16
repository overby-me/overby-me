//! `tangled-spindle` — Rust reimplementation of the Tangled Spindle CI runner.
//!
//! This is the main entry point for the tangled-spindle-nix binary. It wires
//! together all subsystems (HTTP server, Jetstream consumer, knot event consumer,
//! engine, database, RBAC, secrets, job queue) and runs them concurrently.
//!
//! See PLAN.md for the full architecture and phase plan.

mod config;
mod notifier;
mod orchestrator;
mod router;
mod server;

use std::process::ExitCode;

use clap::Parser;
use tracing::{error, info};

/// Tangled Spindle CI Runner — Rust reimplementation with native Nix engine.
#[derive(Parser, Debug)]
#[command(name = "tangled-spindle", version, about)]
struct Cli {
    /// Run in dev mode (overrides SPINDLE_SERVER_DEV).
    #[arg(long, env = "SPINDLE_SERVER_DEV")]
    dev: bool,

    /// Override the listen address.
    #[arg(long, env = "SPINDLE_SERVER_LISTEN_ADDR")]
    listen_addr: Option<String>,

    /// Print the loaded configuration and exit (for debugging).
    #[arg(long)]
    print_config: bool,
}

fn init_tracing(dev: bool) {
    let builder = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                if dev {
                    "tangled_spindle=debug,spindle_db=debug,spindle_rbac=debug,info".into()
                } else {
                    "info".into()
                }
            }),
        )
        .with_target(true)
        .with_thread_ids(false);

    if dev {
        builder.pretty().init();
    } else {
        builder.json().init();
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    init_tracing(cli.dev);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "tangled-spindle-nix starting"
    );

    // Load configuration from environment variables
    let cfg = match config::Config::from_env() {
        Ok(mut cfg) => {
            // Apply CLI overrides
            if cli.dev {
                cfg.dev = true;
            }
            if let Some(addr) = cli.listen_addr {
                cfg.listen_addr = addr;
            }
            cfg
        }
        Err(e) => {
            error!(%e, "failed to load configuration");
            eprintln!("Error: {e}");
            eprintln!();
            eprintln!("Required environment variables:");
            eprintln!("  SPINDLE_SERVER_HOSTNAME    Public hostname of this spindle instance");
            eprintln!("  SPINDLE_SERVER_OWNER       DID of the spindle owner");
            eprintln!(
                "  SPINDLE_SERVER_TOKEN       Authentication token (or SPINDLE_SERVER_TOKEN_FILE)"
            );
            return ExitCode::FAILURE;
        }
    };

    if cli.print_config {
        println!("Configuration:");
        println!("  hostname:            {}", cfg.hostname);
        println!("  did_web:             {}", cfg.did_web);
        println!("  owner:               {}", cfg.owner);
        println!("  listen_addr:         {}", cfg.listen_addr);
        println!("  jetstream_endpoint:  {}", cfg.jetstream_endpoint);
        println!("  plc_url:             {}", cfg.plc_url);
        println!("  db_path:             {}", cfg.db_path.display());
        println!("  log_dir:             {}", cfg.log_dir.display());
        println!("  dev:                 {}", cfg.dev);
        println!("  engine.kind:         {}", cfg.engine.kind);
        println!("  engine.max_jobs:     {}", cfg.engine.max_jobs);
        println!("  engine.queue_size:   {}", cfg.engine.queue_size);
        println!("  engine.timeout:      {:?}", cfg.engine.workflow_timeout);
        println!("  engine.nixery_url:   {}", cfg.engine.nixery_url);
        println!("  secrets.provider:    {}", cfg.secrets.provider);
        return ExitCode::SUCCESS;
    }

    info!(
        hostname = %cfg.hostname,
        did_web = %cfg.did_web,
        owner = %cfg.owner,
        listen_addr = %cfg.listen_addr,
        engine = %cfg.engine.kind,
        dev = cfg.dev,
        "configuration loaded"
    );

    // Initialize database
    let db = match spindle_db::Database::open(&cfg.db_path) {
        Ok(db) => {
            info!(path = %cfg.db_path.display(), "database opened");
            std::sync::Arc::new(db)
        }
        Err(e) => {
            error!(%e, path = %cfg.db_path.display(), "failed to open database");
            return ExitCode::FAILURE;
        }
    };

    // Initialize RBAC enforcer
    let rbac = match spindle_rbac::SpindleEnforcer::new().await {
        Ok(enforcer) => {
            info!("RBAC enforcer initialized");
            enforcer
        }
        Err(e) => {
            error!(%e, "failed to initialize RBAC enforcer");
            return ExitCode::FAILURE;
        }
    };

    // Bootstrap RBAC: register spindle and owner
    if let Err(e) = rbac.add_spindle(&cfg.did_web).await {
        error!(%e, "failed to register spindle in RBAC");
        return ExitCode::FAILURE;
    }
    if let Err(e) = rbac.add_spindle_owner(&cfg.did_web, &cfg.owner).await {
        error!(%e, "failed to register spindle owner in RBAC");
        return ExitCode::FAILURE;
    }

    // Ensure owner is in the database and DID watch list
    if let Err(e) = db.add_spindle_owner(&cfg.owner) {
        error!(%e, "failed to add spindle owner to database");
        return ExitCode::FAILURE;
    }
    if let Err(e) = db.add_did(&cfg.owner) {
        error!(%e, "failed to add owner DID to watch list");
        return ExitCode::FAILURE;
    }

    info!(
        owner = %cfg.owner,
        "spindle bootstrapped — owner registered in RBAC and database"
    );

    // Start all subsystems concurrently (HTTP server, Jetstream consumer,
    // knot consumer, pipeline orchestrator, job queue).
    if let Err(e) = server::run_server(cfg, db, rbac).await {
        error!(%e, "server exited with error");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
