//! `tangled-cli string` — Tangled's paste, stored on your own PDS.

use anyhow::{anyhow, Result};
use std::io::Read;

use crate::cli::{
    Cli, StringCommand, StringCreateArgs, StringDeleteArgs, StringListArgs, StringViewArgs,
};

pub async fn run(_cli: &Cli, cmd: StringCommand) -> Result<()> {
    match cmd {
        StringCommand::List(args) => list(args).await,
        StringCommand::View(args) => view(args).await,
        StringCommand::Create(args) => create(args).await,
        StringCommand::Delete(args) => delete(args).await,
    }
}

/// Whose strings to read: a named handle, or your own.
async fn subject_did(
    client: &tangled_api::TangledClient,
    session: &tangled_config::session::Session,
    user: Option<&str>,
) -> Result<String> {
    match user {
        None => Ok(session.did.clone()),
        Some(u) => client.resolve_handle(u, None).await,
    }
}

async fn list(args: StringListArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);
    let did = subject_did(&client, &session, args.user.as_deref()).await?;

    let strings = client
        .list_strings(&did, Some(session.access_jwt.as_str()))
        .await?;
    if strings.is_empty() {
        println!("No strings");
        return Ok(());
    }
    println!("RKEY\tFILENAME\tLINES\tPREVIEW");
    for s in &strings {
        println!(
            "{}\t{}\t{}\t{}",
            s.rkey,
            s.value.filename,
            s.value.line_count(),
            s.value.preview()
        );
    }
    Ok(())
}

async fn view(args: StringViewArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);
    let did = subject_did(&client, &session, args.user.as_deref()).await?;

    let s = client
        .get_string(&did, &args.rkey, Some(session.access_jwt.as_str()))
        .await?;
    if args.raw {
        // Contents only, so it can be piped into a file.
        print!("{}", s.contents);
        return Ok(());
    }
    println!("FILENAME:    {}", s.filename);
    if !s.description.is_empty() {
        println!("DESCRIPTION: {}", s.description);
    }
    if let Some(created) = &s.created_at {
        println!("CREATED:     {created}");
    }
    println!();
    print!("{}", s.contents);
    if !s.contents.ends_with('\n') {
        println!();
    }
    Ok(())
}

async fn create(args: StringCreateArgs) -> Result<()> {
    // A file, or stdin: a paste tool that cannot be piped into is half a tool.
    let (contents, default_name) = match args.file.as_deref() {
        Some("-") | None
            if args.file.is_some() || !std::io::IsTerminal::is_terminal(&std::io::stdin()) =>
        {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            (buf, "stdin.txt".to_string())
        }
        Some(path) => {
            let contents =
                std::fs::read_to_string(path).map_err(|e| anyhow!("read {path}: {e}"))?;
            let name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "string.txt".into());
            (contents, name)
        }
        None => return Err(anyhow!("give a file, or pipe the contents in on stdin")),
    };
    if contents.trim().is_empty() {
        return Err(anyhow!("refusing to create an empty string"));
    }

    let filename = args.filename.clone().unwrap_or(default_name);
    let description = args.description.clone().unwrap_or_default();

    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);
    let rkey = client
        .create_string(
            &session.did,
            &filename,
            &description,
            &contents,
            &pds,
            &session.access_jwt,
        )
        .await?;
    println!("Created string {rkey} ({filename})");
    println!("  https://tangled.org/{}/strings/{rkey}", session.handle);
    Ok(())
}

async fn delete(args: StringDeleteArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);
    client
        .delete_string(&session.did, &args.rkey, &pds, &session.access_jwt)
        .await?;
    println!("Deleted string {}", args.rkey);
    Ok(())
}
