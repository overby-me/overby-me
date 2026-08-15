use anyhow::{anyhow, Result};
use dialoguer::{Input, Password};

use crate::cli::{AuthCommand, AuthLoginArgs, AuthLoginBrowserArgs, AuthSwitchArgs, Cli};

pub async fn run(cli: &Cli, cmd: AuthCommand) -> Result<()> {
    match cmd {
        AuthCommand::Login(args) => login(cli, args).await,
        AuthCommand::LoginBrowser(args) => login_browser(cli, args).await,
        AuthCommand::Status(args) => status(cli, args.clone()).await,
        AuthCommand::Logout => logout(cli).await,
        AuthCommand::List => list(cli).await,
        AuthCommand::Switch(args) => switch(cli, args.clone()).await,
        AuthCommand::Token => token(cli).await,
    }
}

async fn login(_cli: &Cli, mut args: AuthLoginArgs) -> Result<()> {
    let handle: String = match args.handle.take() {
        Some(h) => h,
        None => Input::new().with_prompt("Handle").interact_text()?,
    };
    let password: String = match args.password.take() {
        Some(p) => p,
        None => Password::new().with_prompt("Password").interact()?,
    };
    let pds = args
        .pds
        .unwrap_or_else(|| "https://bsky.social".to_string());

    let client = tangled_api::TangledClient::new(&pds);
    let mut session = match client.login_with_password(&handle, &password, &pds).await {
        Ok(sess) => sess,
        Err(e) => {
            println!("\x1b[93mIf you're on your own PDS, make sure to pass the --pds flag\x1b[0m");
            return Err(e);
        }
    };
    session.pds = Some(pds.clone());
    crate::util::session_manager().save(&session)?;
    remember_profile()?;
    println!("Logged in as '{}' ({})", session.handle, session.did);
    Ok(())
}

async fn login_browser(_cli: &Cli, args: AuthLoginBrowserArgs) -> Result<()> {
    let input: String = match args.handle {
        Some(h) => h,
        None => Input::new().with_prompt("Handle").interact_text()?,
    };

    println!("Opening browser for authentication...");
    let result = tangled_api::oauth::login_browser(&input).await?;

    // Save the OAuth session for DPoP-authenticated requests
    let oauth_json = serde_json::to_string(&result.persisted)?;
    tangled_config::keychain::Keychain::new("tangled-cli-oauth", "default")
        .set_password(&oauth_json)?;

    // Also save a basic session for compatibility with existing commands
    let session = tangled_config::session::Session {
        access_jwt: String::new(),
        refresh_jwt: String::new(),
        did: result.did.clone(),
        handle: result.handle.clone(),
        pds: result.pds.clone(),
        created_at: chrono::Utc::now(),
    };
    crate::util::session_manager().save(&session)?;
    remember_profile()?;
    println!("Logged in as '{}' ({})", result.handle, result.did);
    Ok(())
}

async fn status(_cli: &Cli, args: crate::cli::AuthStatusArgs) -> Result<()> {
    let mgr = crate::util::session_manager();
    match mgr.load()? {
        Some(s) => {
            println!("Logged in as '{}' ({})", s.handle, s.did);
            if let Some(pds) = &s.pds {
                println!("PDS: {}", pds);
            }
        }
        None => println!("Not logged in. Run: tangled auth login"),
    }
    if args.verify {
        // What is stored says nothing about whether the PDS still accepts it.
        let session = crate::util::load_session_with_refresh().await?;
        let pds = crate::util::pds_of(&session);
        let client = crate::util::make_client(&pds);
        match client.get_session(&session.access_jwt).await {
            Ok(info) => println!(
                "Verified: session is live for {} ({}){}",
                info.handle,
                info.did,
                match info.active {
                    Some(false) => ", account INACTIVE",
                    _ => "",
                }
            ),
            Err(e) => println!("Verify failed: {e}"),
        }
    }
    Ok(())
}

async fn logout(_cli: &Cli) -> Result<()> {
    let mgr = crate::util::session_manager();
    let had_session = mgr.load()?.is_some();
    if had_session {
        mgr.clear()?;
    }

    // `login-browser` writes a second keychain entry. Leaving it behind meant
    // "logout, then log in again with a password" still ran every request
    // through the stale OAuth session, because an OAuth session takes
    // precedence over the password JWTs.
    let oauth = tangled_config::keychain::Keychain::new("tangled-cli-oauth", "default");
    let had_oauth = oauth.get_password().is_ok();
    if had_oauth {
        oauth.delete_password()?;
    }

    if had_session || had_oauth {
        println!("Logged out");
    } else {
        println!("No session found");
    }
    Ok(())
}

/// Record the profile just logged into, and make it active. The keyring
/// cannot be enumerated, so without this list nothing could find it again.
fn remember_profile() -> Result<()> {
    let name = crate::util::active_profile();
    let mut cfg = tangled_config::config::load_config(None)?.unwrap_or_default();
    cfg.profiles.remember(&name);
    tangled_config::config::save_config(&cfg, None)?;
    Ok(())
}

async fn list(_cli: &Cli) -> Result<()> {
    let cfg = tangled_config::config::load_config(None)?.unwrap_or_default();
    let active = crate::util::active_profile();

    // Always include the profile in force: a login made before profiles
    // existed is in the keyring but not in the config.
    let mut names = cfg.profiles.known.clone();
    if !names.iter().any(|n| n == &active) {
        names.insert(0, active.clone());
    }
    if names.is_empty() {
        println!("No accounts. Run: tangled auth login");
        return Ok(());
    }

    println!("ACTIVE\tPROFILE\tHANDLE\tDID");
    for name in names {
        let session = tangled_config::session::SessionManager::new("tangled-cli", &name).load()?;
        let (handle, did) = match session {
            Some(s) => (s.handle, s.did),
            None => ("(no session)".into(), String::new()),
        };
        let marker = if name == active { "*" } else { " " };
        println!("{marker}\t{name}\t{handle}\t{did}");
    }
    Ok(())
}

async fn switch(_cli: &Cli, args: AuthSwitchArgs) -> Result<()> {
    let session =
        tangled_config::session::SessionManager::new("tangled-cli", &args.profile).load()?;
    let Some(session) = session else {
        return Err(anyhow!(
            "profile '{}' has no session; log into it with: tangled auth login --profile {}",
            args.profile,
            args.profile
        ));
    };
    let mut cfg = tangled_config::config::load_config(None)?.unwrap_or_default();
    cfg.profiles.remember(&args.profile);
    tangled_config::config::save_config(&cfg, None)?;
    println!("Switched to '{}' ({})", session.handle, args.profile);
    Ok(())
}

async fn token(_cli: &Cli) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    if session.access_jwt.is_empty() {
        return Err(anyhow!(
            "this session has no access token: it was created by `auth login-browser`, which stores an OAuth session instead"
        ));
    }
    println!("{}", session.access_jwt);
    Ok(())
}
