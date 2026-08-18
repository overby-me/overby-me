//! `tangled-cli ssh-key` — manage the keys a knot will accept a push from.

use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::cli::{Cli, SshKeyAddArgs, SshKeyCommand, SshKeyDeleteArgs, SshKeyListArgs};

/// What `ssh-keygen` produces by default, and so what most people have.
const DEFAULT_KEY: &str = ".ssh/id_ed25519.pub";

pub async fn run(_cli: &Cli, cmd: SshKeyCommand) -> Result<()> {
    match cmd {
        SshKeyCommand::List(args) => list(args).await,
        SshKeyCommand::Add(args) => add(args).await,
        SshKeyCommand::Delete(args) => delete(args).await,
    }
}

async fn list(args: SshKeyListArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);

    let did = match args.user.as_deref() {
        None => session.did.clone(),
        Some(u) if u.starts_with("did:") => u.to_string(),
        Some(handle) => client.resolve_handle(handle, None).await?,
    };

    let keys = client
        .list_public_keys(&did, Some(session.access_jwt.as_str()))
        .await?;
    if keys.is_empty() {
        println!("No SSH keys published");
        return Ok(());
    }
    println!("NAME\tRKEY\tKEY");
    for record in keys {
        println!(
            "{}\t{}\t{}",
            record.key.name,
            record.rkey,
            record.key.summary()
        );
    }
    Ok(())
}

async fn add(args: SshKeyAddArgs) -> Result<()> {
    let path = match args.file.clone() {
        Some(p) => p,
        None => {
            let home =
                dirs::home_dir().ok_or_else(|| anyhow!("cannot find your home directory"))?;
            home.join(DEFAULT_KEY)
        }
    };
    let key = read_public_key(&path)?;

    // Default the name to the file it came from, as tg does: a key named
    // "id_ed25519.pub" is at least traceable, whereas an empty name is not.
    let name = args.name.clone().unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "key".into())
    });

    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);

    let rkey = client
        .add_public_key(&session.did, &name, &key, &pds, &session.access_jwt)
        .await?;
    println!("Added SSH key '{name}' ({rkey})");
    println!("  from {}", path.display());
    Ok(())
}

async fn delete(args: SshKeyDeleteArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);

    let keys = client
        .list_public_keys(&session.did, Some(session.access_jwt.as_str()))
        .await?;
    // Accept either the name or the record key: the name is what `list`
    // leads with, the rkey is what is unique.
    let hits: Vec<_> = keys
        .iter()
        .filter(|r| r.rkey == args.key || r.key.name == args.key)
        .collect();
    let record = match hits.as_slice() {
        [] => return Err(anyhow!("no SSH key named or keyed '{}'", args.key)),
        [one] => *one,
        many => {
            return Err(anyhow!(
                "'{}' matches {} keys; delete by rkey instead: {}",
                args.key,
                many.len(),
                many.iter()
                    .map(|r| r.rkey.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    };

    client
        .delete_public_key(&session.did, &record.rkey, &pds, &session.access_jwt)
        .await?;
    println!("Deleted SSH key '{}' ({})", record.key.name, record.rkey);
    Ok(())
}

/// Read a public key, rejecting the mistake that matters: handing over a
/// private key.
fn read_public_key(path: &PathBuf) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow!("read key file {}: {e}", path.display()))?;
    let key = raw.trim().to_string();
    if key.is_empty() {
        return Err(anyhow!("key file {} is empty", path.display()));
    }
    if key.contains("PRIVATE KEY") {
        return Err(anyhow!(
            "{} is a PRIVATE key; publish the .pub file instead",
            path.display()
        ));
    }
    Ok(key)
}
