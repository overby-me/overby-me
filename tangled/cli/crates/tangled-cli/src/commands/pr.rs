use crate::cli::{
    Cli, PrCheckoutArgs, PrCloseArgs, PrCommand, PrCommentArgs, PrCreateArgs, PrDiffArgs,
    PrListArgs, PrMergeArgs, PrReopenArgs, PrReviewArgs, PrShowArgs,
};
use anyhow::{anyhow, Result};
use std::process::Command;

pub async fn run(_cli: &Cli, cmd: PrCommand) -> Result<()> {
    match cmd {
        PrCommand::List(args) => list(args).await,
        PrCommand::Create(args) => create(args).await,
        PrCommand::Show(args) => show(args).await,
        PrCommand::Review(args) => review(args).await,
        PrCommand::Merge(args) => merge(args).await,
        PrCommand::Comment(args) => comment(args).await,
        PrCommand::Diff(args) => diff(args).await,
        PrCommand::Close(args) => close(args).await,
        PrCommand::Reopen(args) => reopen(args).await,
        PrCommand::Checkout(args) => checkout(args).await,
    }
}

async fn list(args: PrListArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = crate::util::pds_of(&session);
    let client = crate::util::make_client(&pds);

    // As with issues: the appview indexes the whole repo, and reports merged
    // as a state of its own. Without a repo we can only scan your own PDS,
    // which shows pulls you opened and nothing else.
    let Some(repo) = args.repo.as_deref() else {
        let pulls = client
            .list_pulls(&session.did, None, Some(session.access_jwt.as_str()))
            .await?;
        if pulls.is_empty() {
            println!("No pull requests found (showing only those you created)");
        } else {
            println!("RKEY\tTITLE\tTARGET");
            for pr in pulls {
                println!("{}\t{}\t{}", pr.rkey, pr.pull.title, pr.pull.target.repo);
            }
            println!("\nShowing only pulls you opened; pass --repo to list a repo's pulls.");
        }
        return Ok(());
    };

    let (owner, name) = parse_repo_ref(repo, &session.handle);
    let info = client
        .get_repo_info(owner, name, Some(session.access_jwt.as_str()))
        .await?;
    let repo_did = info
        .repo_did
        .clone()
        .ok_or_else(|| anyhow!("{owner}/{name} has no repoDid; recreate it"))?;

    let appview = crate::util::make_client(&tangled_api::appview::appview_base());
    let items = appview
        .list_pulls_indexed(&repo_did, args.state.as_deref(), 1000)
        .await?;
    if items.is_empty() {
        println!("No pull requests");
        return Ok(());
    }
    println!("RKEY\tSTATE\tCOMMENTS\tTITLE");
    for it in &items {
        println!(
            "{}\t{}\t{}\t{}",
            it.rkey(),
            it.state.clone().unwrap_or_default(),
            it.comment_count,
            it.value.title
        );
    }
    Ok(())
}

async fn create(args: PrCreateArgs) -> Result<()> {
    let repo = args
        .repo
        .as_ref()
        .ok_or_else(|| anyhow!("--repo is required for pr create"))?;
    let (owner, name) = parse_repo_ref(repo, "");

    let base = args
        .base
        .as_deref()
        .ok_or_else(|| anyhow!("--base is required (target branch)"))?;
    let head = args
        .head
        .as_deref()
        .ok_or_else(|| anyhow!("--head is required (source range/branch)"))?;

    let title_buf;
    let title = if let Some(t) = args.title.as_deref() {
        t
    } else {
        title_buf = format!("{} -> {}", head, base);
        &title_buf
    };

    // The appview requires a cookie-based OAuth session to create PRs,
    // which cannot be done programmatically from the CLI. Open the
    // browser to the PR creation page with pre-filled parameters.
    let appview_base =
        std::env::var("TANGLED_APPVIEW_BASE").unwrap_or_else(|_| "https://tangled.org".to_string());
    let mut pr_url = url::Url::parse(&format!("{}/{}/{}/pulls/new", appview_base, owner, name))?;
    pr_url
        .query_pairs_mut()
        .append_pair("strategy", "branch")
        .append_pair("sourceBranch", head)
        .append_pair("targetBranch", base)
        .append_pair("title", title);
    if let Some(body) = args.body.as_deref() {
        pr_url.query_pairs_mut().append_pair("body", body);
    }

    println!("Opening PR creation page: {}", pr_url);
    open::that(pr_url.as_str())?;
    Ok(())
}

async fn show(args: PrShowArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (did, rkey) = parse_record_id(&args.id, &session.did)?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let client = crate::util::make_client(&pds);
    let pr = client
        .get_pull_record(&did, &rkey, Some(session.access_jwt.as_str()))
        .await?;
    println!("TITLE: {}", pr.title);
    if !pr.body.is_empty() {
        println!("BODY:\n{}", pr.body);
    }
    println!("TARGET: {} @ {}", pr.target.repo, pr.target.branch);
    if args.diff {
        println!(
            "PATCH:\n{}",
            pr.patch.as_deref().unwrap_or("(no inline patch)")
        );
    }
    Ok(())
}

async fn review(args: PrReviewArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (did, rkey) = parse_record_id(&args.id, &session.did)?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pr_at = format!("at://{}/sh.tangled.repo.pull/{}", did, rkey);
    let note = if let Some(c) = args.comment.as_deref() {
        c
    } else if args.approve {
        "LGTM"
    } else if args.request_changes {
        "Requesting changes"
    } else {
        ""
    };
    if note.is_empty() {
        return Err(anyhow!("provide --comment or --approve/--request-changes"));
    }
    let client = crate::util::make_client(&pds);
    client
        .comment_pull(&session.did, &pr_at, note, &pds, &session.access_jwt)
        .await?;
    println!("Review comment posted");
    Ok(())
}

async fn comment(args: PrCommentArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (did, rkey) = parse_record_id(&args.id, &session.did)?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pr_at = format!("at://{}/sh.tangled.repo.pull/{}", did, rkey);
    let client = crate::util::make_client(&pds);
    client
        .comment_pull(&session.did, &pr_at, &args.body, &pds, &session.access_jwt)
        .await?;
    println!("Comment posted");
    Ok(())
}

async fn diff(args: PrDiffArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (did, rkey) = parse_record_id(&args.id, &session.did)?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let client = crate::util::make_client(&pds);
    let pr = client
        .get_pull_record(&did, &rkey, Some(session.access_jwt.as_str()))
        .await?;
    match pr.patch.as_deref() {
        Some(patch) if !patch.is_empty() => print!("{}", patch),
        _ => println!("(no patch attached to this PR)"),
    }
    Ok(())
}

async fn close(args: PrCloseArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (did, rkey) = parse_record_id(&args.id, &session.did)?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let client = crate::util::make_client(&pds);
    let pr_at = format!("at://{}/sh.tangled.repo.pull/{}", did, rkey);

    if let Some(comment) = args.comment.as_deref() {
        client
            .comment_pull(&session.did, &pr_at, comment, &pds, &session.access_jwt)
            .await?;
    }
    client
        .set_pull_status(
            &session.did,
            &pr_at,
            tangled_api::client::PULL_STATUS_CLOSED,
            &pds,
            &session.access_jwt,
        )
        .await?;
    println!("Closed PR {}:{}", did, rkey);
    Ok(())
}

async fn reopen(args: PrReopenArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (did, rkey) = parse_record_id(&args.id, &session.did)?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let client = crate::util::make_client(&pds);
    let pr_at = format!("at://{}/sh.tangled.repo.pull/{}", did, rkey);

    if let Some(comment) = args.comment.as_deref() {
        client
            .comment_pull(&session.did, &pr_at, comment, &pds, &session.access_jwt)
            .await?;
    }
    client
        .set_pull_status(
            &session.did,
            &pr_at,
            tangled_api::client::PULL_STATUS_OPEN,
            &pds,
            &session.access_jwt,
        )
        .await?;
    println!("Reopened PR {}:{}", did, rkey);
    Ok(())
}

async fn checkout(args: PrCheckoutArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (did, rkey) = parse_record_id(&args.id, &session.did)?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let client = crate::util::make_client(&pds);
    let pr = client
        .get_pull_record(&did, &rkey, Some(session.access_jwt.as_str()))
        .await?;

    let patch = pr
        .patch
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| anyhow!("PR has no patch to apply"))?;

    let branch_name = args.branch.unwrap_or_else(|| format!("pr/{}", rkey));

    // Create and checkout the branch from the PR's target branch
    let target_branch = &pr.target.branch;

    let status = Command::new("git")
        .args(["checkout", "-b", &branch_name, target_branch])
        .status()?;
    if !status.success() {
        // Branch might already exist, try switching to it
        let status = Command::new("git")
            .args(["checkout", &branch_name])
            .status()?;
        if !status.success() {
            return Err(anyhow!(
                "failed to create or switch to branch '{}'",
                branch_name
            ));
        }
    }

    // Apply the patch via git am
    let mut child = Command::new("git")
        .args(["am", "--3way"])
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(patch.as_bytes())?;
    }

    let exit = child.wait()?;
    if !exit.success() {
        println!("Patch did not apply cleanly. Resolve conflicts and run 'git am --continue'.");
    } else {
        println!("Checked out PR {} on branch '{}'", rkey, branch_name);
    }

    Ok(())
}

async fn merge(args: PrMergeArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let (did, rkey) = parse_record_id(&args.id, &session.did)?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());

    // Get the PR
    let pds_client = crate::util::make_client(&pds);
    let pull = pds_client
        .get_pull_record(&did, &rkey, Some(session.access_jwt.as_str()))
        .await?;

    // Parse target repo info
    let (repo_did, repo_name, knot) = parse_target_repo_info(&pull, &pds_client, &session).await?;

    // Check if PR is part of a stack
    if let Some(stack_id) = &pull.stack_id {
        merge_stacked_pr(
            &pds_client,
            &session,
            &pull,
            &did,
            &rkey,
            &repo_did,
            &repo_name,
            &knot,
            stack_id,
            &pds,
        )
        .await?;
    } else {
        // Single PR merge (existing logic)
        merge_single_pr(&session, &did, &rkey, &repo_did, &repo_name, &knot, &pds).await?;
    }

    Ok(())
}

fn parse_repo_ref<'a>(spec: &'a str, default_owner: &'a str) -> (&'a str, &'a str) {
    if let Some((owner, name)) = spec.split_once('/') {
        if !owner.is_empty() {
            (owner, name)
        } else {
            (default_owner, name)
        }
    } else {
        (default_owner, spec)
    }
}

fn parse_record_id<'a>(id: &'a str, default_did: &'a str) -> Result<(String, String)> {
    if let Some(rest) = id.strip_prefix("at://") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 4 {
            return Ok((parts[0].to_string(), parts[3].to_string()));
        }
    }
    if let Some((did, rkey)) = id.split_once(':') {
        return Ok((did.to_string(), rkey.to_string()));
    }
    Ok((default_did.to_string(), id.to_string()))
}

// Helper functions for stacked PR merge support

async fn merge_single_pr(
    session: &tangled_config::session::Session,
    did: &str,
    rkey: &str,
    repo_did: &str,
    repo_name: &str,
    knot: &str,
    pds: &str,
) -> Result<()> {
    let api = crate::util::make_default_client();
    api.merge_pull(
        did,
        rkey,
        repo_did,
        repo_name,
        knot,
        pds,
        &session.access_jwt,
    )
    .await?;

    println!("Merged PR {}:{}", did, rkey);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn merge_stacked_pr(
    pds_client: &tangled_api::TangledClient,
    session: &tangled_config::session::Session,
    current_pull: &tangled_api::Pull,
    current_did: &str,
    current_rkey: &str,
    repo_did: &str,
    repo_name: &str,
    knot: &str,
    stack_id: &str,
    pds: &str,
) -> Result<()> {
    // Step 1: Get full stack
    println!("🔍 Detecting stack...");
    let stack = get_stack_pulls(pds_client, &session.did, stack_id, &session.access_jwt).await?;

    if stack.is_empty() {
        return Err(anyhow!("Stack is empty"));
    }

    // Step 2: Find substack (current PR and all below it)
    let substack = find_substack(&stack, current_pull.change_id.as_deref())?;

    println!(
        "✓ Detected PR is part of stack (stack has {} total PRs)",
        stack.len()
    );
    println!();
    println!("The following {} PR(s) will be merged:", substack.len());

    for (idx, pr) in substack.iter().enumerate() {
        let marker = if pr.rkey == current_rkey {
            " (current)"
        } else {
            ""
        };
        println!("  [{}] {}: {}{}", idx + 1, pr.rkey, pr.pull.title, marker);
    }
    println!();

    // Step 3: Check for conflicts
    println!("✓ Checking for conflicts...");
    let api = crate::util::make_default_client();
    let conflicts = check_stack_conflicts(
        &api,
        repo_did,
        repo_name,
        &current_pull.target.branch,
        &substack,
        knot,
        pds,
        &session.access_jwt,
    )
    .await?;

    if !conflicts.is_empty() {
        println!("✗ Cannot merge: conflicts detected");
        println!();
        for (pr_rkey, conflict_resp) in conflicts {
            println!(
                "  PR {}: Conflicts in {} file(s)",
                pr_rkey,
                conflict_resp.conflicts.len()
            );
            for conflict in conflict_resp.conflicts {
                println!("    - {}: {}", conflict.filename, conflict.reason);
            }
        }
        return Err(anyhow!("Stack has merge conflicts"));
    }

    println!("✓ All PRs can be merged cleanly");
    println!();

    // Step 4: Confirmation prompt
    if !prompt_confirmation(&format!("Merge {} pull request(s)?", substack.len()))? {
        println!("Merge cancelled.");
        return Ok(());
    }

    // Step 5: Merge the stack (backend handles combined patch)
    println!("Merging {} PR(s)...", substack.len());

    // Use the current PR's merge endpoint - backend will handle the stack
    api.merge_pull(
        current_did,
        current_rkey,
        repo_did,
        repo_name,
        knot,
        pds,
        &session.access_jwt,
    )
    .await?;

    println!("✓ Successfully merged {} pull request(s)", substack.len());

    Ok(())
}

async fn get_stack_pulls(
    client: &tangled_api::TangledClient,
    user_did: &str,
    stack_id: &str,
    bearer: &str,
) -> Result<Vec<tangled_api::PullRecord>> {
    // List all user's PRs and filter by stack_id
    let all_pulls = client.list_pulls(user_did, None, Some(bearer)).await?;

    let mut stack_pulls: Vec<_> = all_pulls
        .into_iter()
        .filter(|p| p.pull.stack_id.as_deref() == Some(stack_id))
        .collect();

    // Order by parent relationships (top to bottom)
    order_stack(&mut stack_pulls)?;

    Ok(stack_pulls)
}

fn order_stack(pulls: &mut Vec<tangled_api::PullRecord>) -> Result<()> {
    if pulls.is_empty() {
        return Ok(());
    }

    // Build parent map: parent_change_id -> pull
    let mut change_id_map: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut parent_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (idx, pr) in pulls.iter().enumerate() {
        if let Some(cid) = &pr.pull.change_id {
            change_id_map.insert(cid.clone(), idx);
        }
        if let Some(pcid) = &pr.pull.parent_change_id {
            parent_map.insert(pcid.clone(), idx);
        }
    }

    // Find top of stack (not a parent of any other PR)
    let mut top_idx = None;
    for (idx, pr) in pulls.iter().enumerate() {
        if let Some(cid) = &pr.pull.change_id {
            if !parent_map.contains_key(cid) {
                top_idx = Some(idx);
                break;
            }
        }
    }

    let top_idx = top_idx.ok_or_else(|| anyhow!("Could not find top of stack"))?;

    // Walk down the stack to build ordered list
    let mut ordered = Vec::new();
    let mut current_idx = top_idx;
    let mut visited = std::collections::HashSet::new();

    loop {
        if visited.contains(&current_idx) {
            return Err(anyhow!("Circular dependency in stack"));
        }
        visited.insert(current_idx);
        ordered.push(current_idx);

        // Find child (PR that has this PR as parent)
        let current_parent = &pulls[current_idx].pull.parent_change_id;
        if current_parent.is_none() {
            break;
        }

        let next_idx = change_id_map.get(current_parent.as_ref().unwrap());

        if let Some(&next) = next_idx {
            current_idx = next;
        } else {
            break;
        }
    }

    // Reorder pulls based on ordered indices
    let original = pulls.clone();
    pulls.clear();
    for idx in ordered {
        pulls.push(original[idx].clone());
    }

    Ok(())
}

fn find_substack<'a>(
    stack: &'a [tangled_api::PullRecord],
    current_change_id: Option<&str>,
) -> Result<Vec<&'a tangled_api::PullRecord>> {
    let change_id = current_change_id.ok_or_else(|| anyhow!("PR has no change_id"))?;

    let position = stack
        .iter()
        .position(|p| p.pull.change_id.as_deref() == Some(change_id))
        .ok_or_else(|| anyhow!("PR not found in stack"))?;

    // Return from current position to end (including current)
    Ok(stack[position..].iter().collect())
}

#[allow(clippy::too_many_arguments)]
async fn check_stack_conflicts(
    api: &tangled_api::TangledClient,
    repo_did: &str,
    repo_name: &str,
    target_branch: &str,
    substack: &[&tangled_api::PullRecord],
    knot: &str,
    pds: &str,
    access_jwt: &str,
) -> Result<Vec<(String, tangled_api::MergeCheckResponse)>> {
    let mut conflicts = Vec::new();
    let mut cumulative_patch = String::new();

    // Check each PR in order (bottom to top of substack)
    for pr in substack.iter().rev() {
        cumulative_patch.push_str(pr.pull.patch.as_deref().unwrap_or(""));
        cumulative_patch.push('\n');

        let check = api
            .merge_check(
                repo_did,
                repo_name,
                target_branch,
                &cumulative_patch,
                knot,
                pds,
                access_jwt,
            )
            .await?;

        if check.is_conflicted {
            conflicts.push((pr.rkey.clone(), check));
        }
    }

    Ok(conflicts)
}

fn prompt_confirmation(message: &str) -> Result<bool> {
    use std::io::{self, Write};

    print!("{} [y/N]: ", message);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}

async fn parse_target_repo_info(
    pull: &tangled_api::Pull,
    pds_client: &tangled_api::TangledClient,
    session: &tangled_config::session::Session,
) -> Result<(String, String, String)> {
    let target_repo = &pull.target.repo;
    let parts: Vec<&str> = target_repo
        .strip_prefix("at://")
        .unwrap_or(target_repo)
        .split('/')
        .collect();

    if parts.len() < 4 {
        return Err(anyhow!("Invalid target repo AT-URI: {}", target_repo));
    }

    let repo_did = parts[0].to_string();
    let repo_rkey = parts[3];

    // Get repo name and knot
    #[derive(serde::Deserialize)]
    struct Rec {
        name: String,
        knot: String,
    }
    #[derive(serde::Deserialize)]
    struct GetRes {
        value: Rec,
    }

    let params = [
        ("repo", repo_did.clone()),
        ("collection", "sh.tangled.repo".to_string()),
        ("rkey", repo_rkey.to_string()),
    ];

    let repo_rec: GetRes = pds_client
        .get_json(
            "com.atproto.repo.getRecord",
            &params,
            Some(&session.access_jwt),
        )
        .await?;

    Ok((repo_did, repo_rec.value.name, repo_rec.value.knot))
}
