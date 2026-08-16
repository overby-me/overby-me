use crate::cli::Cli;
use anyhow::Result;

pub async fn run(_cli: &Cli) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let client = crate::util::make_client(&pds);

    let issues = client
        .list_issues(&session.did, None, Some(session.access_jwt.as_str()))
        .await
        .unwrap_or_default();

    let pulls = client
        .list_pulls(&session.did, None, Some(session.access_jwt.as_str()))
        .await
        .unwrap_or_default();

    let open_issues: Vec<_> = issues.iter().collect();
    let open_pulls: Vec<_> = pulls.iter().collect();

    if open_issues.is_empty() && open_pulls.is_empty() {
        println!("Nothing needs your attention.");
        return Ok(());
    }

    if !open_issues.is_empty() {
        println!("Your issues ({}):", open_issues.len());
        for it in &open_issues {
            println!("  {}  {}", it.rkey, it.issue.title);
        }
        println!();
    }

    if !open_pulls.is_empty() {
        println!("Your pull requests ({}):", open_pulls.len());
        for pr in &open_pulls {
            println!("  {}  {}", pr.rkey, pr.pull.title);
        }
    }

    Ok(())
}
