use anyhow::Result;
use dialoguer::{Input, Password};
use tangled_config::session::SessionManager;

use crate::cli::{AuthCommand, AuthLoginArgs, AuthLoginBrowserArgs, Cli};

pub async fn run(cli: &Cli, cmd: AuthCommand) -> Result<()> {
    match cmd {
        AuthCommand::Login(args) => login(cli, args).await,
        AuthCommand::LoginBrowser(args) => login_browser(cli, args).await,
        AuthCommand::Status => status(cli).await,
        AuthCommand::Logout => logout(cli).await,
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
    SessionManager::default().save(&session)?;
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
    SessionManager::default().save(&session)?;
    println!("Logged in as '{}' ({})", result.handle, result.did);
    Ok(())
}

async fn status(_cli: &Cli) -> Result<()> {
    let mgr = SessionManager::default();
    match mgr.load()? {
        Some(s) => {
            println!("Logged in as '{}' ({})", s.handle, s.did);
            if let Some(pds) = &s.pds {
                println!("PDS: {}", pds);
            }
        }
        None => println!("Not logged in. Run: tangled auth login"),
    }
    Ok(())
}

async fn logout(_cli: &Cli) -> Result<()> {
    let mgr = SessionManager::default();
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
