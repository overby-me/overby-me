use crate::cli::{BrowseArgs, Cli};
use anyhow::{anyhow, Result};

pub async fn run(_cli: &Cli, args: BrowseArgs) -> Result<()> {
    let url = build_url(&args).await?;

    if args.no_browser {
        println!("{}", url);
    } else {
        open::that(&url).map_err(|e| anyhow!("failed to open browser: {e}"))?;
        println!("Opened {}", url);
    }
    Ok(())
}

async fn build_url(args: &BrowseArgs) -> Result<String> {
    let base = "https://tngl.sh";

    let target = match &args.target {
        Some(t) => t.clone(),
        None => detect_repo_from_git()?,
    };

    // If target looks like owner/repo
    if let Some((owner, name)) = target.split_once('/') {
        let owner = owner.strip_prefix('@').unwrap_or(owner);
        let mut url = format!("{}/{}/{}", base, owner, name);
        if args.issues {
            url.push_str("/issues");
        } else if args.prs {
            url.push_str("/pulls");
        }
        return Ok(url);
    }

    // Bare target — could be a repo name for the current user
    let session = crate::util::load_session_with_refresh().await?;
    let handle = &session.handle;
    let mut url = format!("{}/{}/{}", base, handle, target);
    if args.issues {
        url.push_str("/issues");
    } else if args.prs {
        url.push_str("/pulls");
    }
    Ok(url)
}

fn detect_repo_from_git() -> Result<String> {
    let repo = git2::Repository::discover(".")
        .map_err(|_| anyhow!("not in a git repository; provide a target (e.g. owner/repo)"))?;
    let remote = repo
        .find_remote("origin")
        .map_err(|_| anyhow!("no 'origin' remote found; provide a target"))?;
    let url = remote
        .url()
        .ok_or_else(|| anyhow!("origin remote has no URL"))?;
    parse_tangled_remote(url)
}

fn parse_tangled_remote(url: &str) -> Result<String> {
    // SSH: git@tangled.org:owner/repo or git@knot1.tangled.sh:owner/repo
    if let Some(rest) = url.strip_prefix("git@") {
        if let Some((_host, path)) = rest.split_once(':') {
            let path = path.trim_end_matches(".git");
            if let Some((owner, name)) = path.split_once('/') {
                return Ok(format!("{}/{}", owner, name));
            }
        }
    }
    // HTTPS: tangled.org/@owner/repo
    if url.contains("tangled.org") || url.contains("tngl.sh") {
        if let Some(path) = url
            .split("tangled.org/")
            .nth(1)
            .or(url.split("tngl.sh/").nth(1))
        {
            let path = path.trim_end_matches(".git");
            let path = path.strip_prefix('@').unwrap_or(path);
            if let Some((owner, name)) = path.split_once('/') {
                return Ok(format!("{}/{}", owner, name));
            }
        }
    }
    Err(anyhow!(
        "could not parse tangled remote from '{}'; provide a target",
        url
    ))
}
