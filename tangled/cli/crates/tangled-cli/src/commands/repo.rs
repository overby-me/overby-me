use anyhow::{anyhow, Result};
use git2::{build::RepoBuilder, Cred, FetchOptions, RemoteCallbacks};
use serde_json;
use std::path::PathBuf;

use crate::cli::{
    Cli, OutputFormat, RepoCloneArgs, RepoCommand, RepoCreateArgs, RepoDeleteArgs, RepoEditArgs,
    RepoForkArgs, RepoInfoArgs, RepoListArgs, RepoRefArgs, RepoSearchArgs,
    RepoSetDefaultBranchArgs,
};

pub async fn run(cli: &Cli, cmd: RepoCommand) -> Result<()> {
    match cmd {
        RepoCommand::List(args) => list(cli, args).await,
        RepoCommand::Create(args) => create(args).await,
        RepoCommand::Clone(args) => clone(args).await,
        RepoCommand::Info(args) => info(args).await,
        RepoCommand::Edit(args) => edit(args).await,
        RepoCommand::Delete(args) => delete(args).await,
        RepoCommand::Star(args) => star(args).await,
        RepoCommand::Unstar(args) => unstar(args).await,
        RepoCommand::Fork(args) => fork(args).await,
        RepoCommand::SetDefaultBranch(args) => set_default_branch(args).await,
        RepoCommand::Search(args) => search(args).await,
    }
}

async fn list(cli: &Cli, args: RepoListArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;

    // The appview indexes every repo an account owns, wherever the records
    // live, and paginates. Scanning a single PDS with listRecords saw only
    // what that PDS held, stopped at 100, and failed the whole response on
    // one record that would not deserialise.
    let pds = crate::util::pds_of(&session);
    let pds_client = crate::util::make_client(&pds);
    let effective_user = args.user.as_deref().unwrap_or(session.handle.as_str());
    let owner_did = pds_client
        .resolve_handle(effective_user, Some(session.access_jwt.as_str()))
        .await?;

    let appview = crate::util::make_client(&tangled_api::appview::appview_base());
    let mut repos = appview.list_repos_indexed(&owner_did, 1000).await?;
    if let Some(knot) = args.knot.as_deref() {
        repos.retain(|r| r.value.knot.as_deref() == Some(knot));
    }

    match cli.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&repos.iter().map(|r| {
                serde_json::json!({
                    "uri": r.uri,
                    "name": r.value.name.clone().unwrap_or_else(|| r.rkey().to_string()),
                    "knot": r.value.knot,
                    "description": r.value.description,
                    "repoDid": r.value.repo_did,
                })
            }).collect::<Vec<_>>())?);
        }
        OutputFormat::Table => {
            if repos.is_empty() {
                println!("No repositories");
                return Ok(());
            }
            println!("NAME\tKNOT\tDESCRIPTION");
            for r in &repos {
                // A record from before the name field is named by its rkey.
                let name = r.value.name.clone().unwrap_or_else(|| r.rkey().to_string());
                println!(
                    "{}\t{}\t{}",
                    name,
                    r.value.knot.clone().unwrap_or_default(),
                    r.value.description.clone().unwrap_or_default()
                );
            }
        }
    }

    Ok(())
}

async fn create(args: RepoCreateArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;

    let base = std::env::var("TANGLED_API_BASE").unwrap_or_else(|_| "https://tngl.sh".into());
    let client = crate::util::make_client(&base);

    // Determine PDS base and target knot hostname
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let knot = args.knot.unwrap_or_else(|| "tngl.sh".to_string());

    let opts = tangled_api::client::CreateRepoOptions {
        did: &session.did,
        name: &args.name,
        knot: &knot,
        description: args.description.as_deref(),
        default_branch: None,
        source: None,
        source_at: None,
        pds_base: &pds,
        access_jwt: &session.access_jwt,
    };
    client.create_repo(opts).await?;

    println!("Created repo '{}' (knot: {})", args.name, knot);
    Ok(())
}

async fn clone(args: RepoCloneArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;

    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let info = pds_client
        .get_repo_info(owner, &name, Some(session.access_jwt.as_str()))
        .await?;

    let remote = if args.https {
        let owner_path = if owner.starts_with('@') {
            owner.to_string()
        } else {
            format!("@{}", owner)
        };
        format!("https://tangled.org/{}/{}", owner_path, name)
    } else {
        let knot = if info.knot == "knot1.tangled.sh" {
            "tangled.org".to_string()
        } else {
            info.knot.clone()
        };
        format!("git@{}:{}/{}", knot, owner.trim_start_matches('@'), name)
    };

    let target = PathBuf::from(&name);
    println!("Cloning {} -> {:?}", remote, target);

    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, username_from_url, _allowed| {
        if let Some(user) = username_from_url {
            Cred::ssh_key_from_agent(user)
        } else {
            Cred::default()
        }
    });
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    if let Some(d) = args.depth {
        fetch_opts.depth(d as i32);
    }
    let mut builder = RepoBuilder::new();
    builder.fetch_options(fetch_opts);
    match builder.clone(&remote, &target) {
        Ok(_) => Ok(()),
        Err(e) => {
            println!("Failed to clone via libgit2: {}", e);
            println!(
                "Hint: try: git clone{} {}",
                args.depth
                    .map(|d| format!(" --depth {}", d))
                    .unwrap_or_default(),
                remote
            );
            Err(anyhow!(e.to_string()))
        }
    }
}

async fn info(args: RepoInfoArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let info = pds_client
        .get_repo_info(owner, &name, Some(session.access_jwt.as_str()))
        .await?;

    println!("NAME:        {}", info.name);
    if let Some(repo_did) = info.repo_did.as_deref() {
        let appview = crate::util::make_client(&tangled_api::appview::appview_base());
        if let Ok(stars) = appview.count_stars(repo_did).await {
            println!("STARS:       {}", stars.count);
        }
    }
    println!("OWNER DID:   {}", info.did);
    println!("KNOT:        {}", info.knot);
    if let Some(spindle) = info.spindle.as_deref() {
        if !spindle.is_empty() {
            println!("SPINDLE:     {}", spindle);
        }
    }
    if let Some(desc) = info.description.as_deref() {
        if !desc.is_empty() {
            println!("DESCRIPTION: {}", desc);
        }
    }

    let knot_host = if info.knot == "knot1.tangled.sh" {
        "tangled.org".to_string()
    } else {
        info.knot.clone()
    };
    if args.stats {
        let client = crate::util::make_default_client();
        if let Ok(def) = client
            .get_default_branch(&knot_host, &info.did, &info.name)
            .await
        {
            println!(
                "DEFAULT BRANCH: {} ({})",
                def.name,
                def.short_hash.unwrap_or(def.hash)
            );
            if let Some(msg) = def.message {
                if !msg.is_empty() {
                    println!("LAST COMMIT:   {}", msg);
                }
            }
        }
        if let Ok(langs) = client
            .get_languages(&knot_host, &info.did, &info.name)
            .await
        {
            if !langs.languages.is_empty() {
                println!("LANGUAGES:");
                for l in langs.languages.iter().take(6) {
                    println!("  - {} ({}%)", l.name, l.percentage);
                }
            }
        }
    }

    if args.contributors {
        println!("Contributors: not implemented yet");
    }
    Ok(())
}

async fn edit(args: RepoEditArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let info = pds_client
        .get_repo_info(owner, &name, Some(session.access_jwt.as_str()))
        .await?;

    pds_client
        .edit_repo(
            &info.did,
            &info.rkey,
            args.description.as_deref(),
            if args.private {
                Some(true)
            } else if args.public {
                Some(false)
            } else {
                None
            },
            Some(session.access_jwt.as_str()),
        )
        .await?;
    println!("Updated repo '{}'", name);
    Ok(())
}

async fn delete(args: RepoDeleteArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let record = pds_client
        .get_repo_info(owner, &name, Some(session.access_jwt.as_str()))
        .await?;
    let did = record.did;
    let api = crate::util::make_default_client();
    api.delete_repo(&did, &name, args.force, &pds, &session.access_jwt)
        .await?;
    println!("Deleted repo '{}'", name);
    Ok(())
}

async fn star(args: RepoRefArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let info = pds_client
        .get_repo_info(owner, &name, Some(session.access_jwt.as_str()))
        .await?;
    let repo_did = info
        .repo_did
        .clone()
        .ok_or_else(|| anyhow!("{owner}/{name} has no repoDid; recreate it"))?;
    let api = crate::util::make_default_client();
    api.star_repo(&pds, &session.access_jwt, &repo_did, &session.did)
        .await?;
    println!("Starred {}/{}", owner, name);
    Ok(())
}

async fn unstar(args: RepoRefArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let info = pds_client
        .get_repo_info(owner, &name, Some(session.access_jwt.as_str()))
        .await?;
    let repo_did = info
        .repo_did
        .clone()
        .ok_or_else(|| anyhow!("{owner}/{name} has no repoDid; recreate it"))?;
    let api = crate::util::make_default_client();
    api.unstar_repo(
        &pds,
        &session.access_jwt,
        &repo_did,
        &format!("at://{}/sh.tangled.repo/{}", info.did, info.rkey),
        &session.did,
    )
    .await?;
    println!("Unstarred {}/{}", owner, name);
    Ok(())
}

async fn fork(args: RepoForkArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);

    let (owner, source_name) = parse_repo_ref(&args.repo, &session.handle);
    let info = pds_client
        .get_repo_info(owner, &source_name, Some(session.access_jwt.as_str()))
        .await?;

    let fork_name = args.name.unwrap_or_else(|| source_name.clone());
    let knot = args.knot.unwrap_or_else(|| info.knot.clone());

    // Build HTTPS source URL for the knot to clone (knot uses DID-based paths)
    let source_url = format!("https://{}/{}/{}", info.knot, info.did, source_name);

    // AT URI of the source repo record (marks this as a fork in the PDS)
    let source_at = format!("at://{}/sh.tangled.repo/{}", info.did, info.rkey);

    let api_client = crate::util::make_default_client();

    let opts = tangled_api::client::CreateRepoOptions {
        did: &session.did,
        name: &fork_name,
        knot: &knot,
        description: info.description.as_deref(),
        default_branch: None,
        source: Some(&source_url),
        source_at: Some(&source_at),
        pds_base: &pds,
        access_jwt: &session.access_jwt,
    };
    api_client.create_repo(opts).await?;

    println!(
        "Forked {}/{} -> {}/{} (knot: {})",
        owner, source_name, session.handle, fork_name, knot
    );
    Ok(())
}

fn parse_repo_ref<'a>(spec: &'a str, default_owner: &'a str) -> (&'a str, String) {
    if let Some((owner, name)) = spec.split_once('/') {
        (owner, name.to_string())
    } else {
        (default_owner, spec.to_string())
    }
}

async fn set_default_branch(args: RepoSetDefaultBranchArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let (owner, name) = (owner.to_string(), name.to_string());
    let info = client
        .get_repo_info(&owner, &name, Some(session.access_jwt.as_str()))
        .await?;
    // The knot keys this on the repo's own DID, as it does for a push.
    let repo_did = info
        .repo_did
        .clone()
        .ok_or_else(|| anyhow!("{owner}/{name} has no repoDid; recreate it"))?;
    client
        .set_default_branch(
            &info.knot,
            &repo_did,
            &args.branch,
            &pds,
            &session.access_jwt,
        )
        .await?;
    println!("Default branch of {owner}/{name} is now {}", args.branch);
    Ok(())
}

async fn search(args: RepoSearchArgs) -> Result<()> {
    let appview = crate::util::make_client(&tangled_api::appview::appview_base());
    let result = appview.search(&args.query, args.limit).await?;
    let hits: Vec<_> = result
        .hits
        .iter()
        .filter(|h| match args.kind.as_deref() {
            Some(k) => h.kind() == k || h.nsid == k,
            None => true,
        })
        .collect();
    if hits.is_empty() {
        println!("No matches");
        return Ok(());
    }
    println!("KIND\tSCORE\tTITLE");
    for h in hits {
        println!("{}\t{:.1}\t{}", h.kind(), h.score, h.title());
    }
    Ok(())
}
