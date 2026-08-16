use crate::cli::{Cli, KnotCommand, KnotMigrateArgs};
use anyhow::anyhow;
use anyhow::Result;
use git2::{Direction, Repository as GitRepository, StatusOptions};
use std::path::Path;

pub async fn run(_cli: &Cli, cmd: KnotCommand) -> Result<()> {
    match cmd {
        KnotCommand::List(args) => list(args).await,
        KnotCommand::Migrate(args) => migrate(args).await,
    }
}

async fn migrate(args: KnotMigrateArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    // 1) Ensure we're inside a git repository and working tree is clean
    let repo = GitRepository::discover(Path::new("."))?;
    let mut status_opts = StatusOptions::new();
    status_opts.include_untracked(false).include_ignored(false);
    let statuses = repo.statuses(Some(&mut status_opts))?;
    if !statuses.is_empty() {
        return Err(anyhow!(
            "working tree has uncommitted changes; commit/push before migrating"
        ));
    }

    // 2) Derive current branch and ensure it's pushed to origin
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Err(anyhow!("repository does not have a HEAD")),
    };
    let head_oid = head
        .target()
        .ok_or_else(|| anyhow!("failed to resolve HEAD OID"))?;
    let head_name = head.shorthand().unwrap_or("");
    let full_ref = head.name().unwrap_or("").to_string();
    if !full_ref.starts_with("refs/heads/") {
        return Err(anyhow!(
            "HEAD is detached; please checkout a branch before migrating"
        ));
    }
    let branch = head_name.to_string();

    let origin = repo.find_remote("origin").or_else(|_| {
        repo.remotes().and_then(|rems| {
            rems.get(0)
                .ok_or(git2::Error::from_str("no remotes configured"))
                .and_then(|name| repo.find_remote(name))
        })
    })?;

    // Connect and list remote heads to find refs/heads/<branch>
    let mut remote = origin;
    remote.connect(Direction::Fetch)?;
    let remote_heads = remote.list()?;
    let remote_oid = remote_heads
        .iter()
        .find_map(|h| {
            if h.name() == format!("refs/heads/{}", branch) {
                Some(h.oid())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("origin does not have branch '{}' — push first", branch))?;
    if remote_oid != head_oid {
        return Err(anyhow!(
            "local {} ({}) != origin {} ({}); please push before migrating",
            branch,
            head_oid,
            branch,
            remote_oid
        ));
    }

    // 3) Parse origin URL to verify repo identity
    let origin_url = remote
        .url()
        .ok_or_else(|| anyhow!("origin has no URL"))?
        .to_string();
    let (origin_owner, origin_name, _origin_host) = parse_remote_url(&origin_url)
        .ok_or_else(|| anyhow!("unsupported origin URL: {}", origin_url))?;

    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    if origin_owner.trim_start_matches('@') != owner.trim_start_matches('@') || origin_name != name
    {
        return Err(anyhow!(
            "repo mismatch: current checkout '{}'/{} != argument '{}'/{}",
            origin_owner,
            origin_name,
            owner,
            name
        ));
    }

    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let info = pds_client
        .get_repo_info(owner, &name, Some(session.access_jwt.as_str()))
        .await?;

    // Build a publicly accessible source URL on tangled.org for the existing repo
    let owner_path = if owner.starts_with('@') {
        owner.to_string()
    } else {
        format!("@{}", owner)
    };
    let source = if args.https {
        format!("https://tangled.org/{}/{}", owner_path, name)
    } else {
        format!(
            "git@{}:{}/{}",
            info.knot,
            owner.trim_start_matches('@'),
            name
        )
    };

    // Create the repo on the target knot, seeding from source
    let client = crate::util::make_default_client();
    let opts = tangled_api::client::CreateRepoOptions {
        did: &session.did,
        name: &name,
        knot: &args.to,
        description: info.description.as_deref(),
        default_branch: None,
        source: Some(&source),
        source_at: None,
        pds_base: &pds,
        access_jwt: &session.access_jwt,
    };
    client.create_repo(opts).await?;

    // Update the PDS record to point to the new knot
    if args.update_record {
        client
            .update_repo_knot(
                &session.did,
                &info.rkey,
                &args.to,
                &pds,
                &session.access_jwt,
            )
            .await?;
    }

    println!("Migrated repo '{}' to knot {}", name, args.to);
    println!(
        "Note: old repository on {} is not deleted automatically.",
        info.knot
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

fn parse_remote_url(url: &str) -> Option<(String, String, String)> {
    // Returns (owner, name, host)
    if let Some(rest) = url.strip_prefix("git@") {
        // git@host:owner/name(.git)
        let mut parts = rest.split(':');
        let host = parts.next()?.to_string();
        let path = parts.next()?;
        let mut segs = path.trim_end_matches(".git").split('/');
        let owner = segs.next()?.to_string();
        let name = segs.next()?.to_string();
        return Some((owner, name, host));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        if let Ok(parsed) = url::Url::parse(url) {
            let host = parsed.host_str().unwrap_or("").to_string();
            let path = parsed.path().trim_matches('/');
            // paths may be like '@owner/name' or 'owner/name'
            let mut segs = path.trim_end_matches(".git").split('/');
            let first = segs.next()?;
            let owner = first.trim_start_matches('@').to_string();
            let name = segs.next()?.to_string();
            return Some((owner, name, host));
        }
    }
    None
}

async fn list(args: crate::cli::KnotListArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);
    let did = match args.user.as_deref() {
        None => session.did.clone(),
        Some(u) => client.resolve_handle(u, None).await?,
    };
    let knots = client
        .list_knots(&did, Some(session.access_jwt.as_str()))
        .await?;
    if knots.is_empty() {
        // Registering a knot means running one; using the shared knot needs
        // no record at all.
        println!("No knots registered (this account uses a shared knot)");
        return Ok(());
    }
    println!("KNOT\tREGISTERED");
    for (host, created) in knots {
        println!("{host}\t{created}");
    }
    Ok(())
}
