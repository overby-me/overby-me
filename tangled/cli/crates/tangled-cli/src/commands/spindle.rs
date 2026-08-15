use crate::cli::{
    Cli, SpindleCancelArgs, SpindleCommand, SpindleConfigArgs, SpindleListArgs, SpindleLogsArgs,
    SpindleRunArgs, SpindleRunsArgs, SpindleSecretAddArgs, SpindleSecretCommand,
    SpindleSecretListArgs, SpindleSecretRemoveArgs, SpindleStatusArgs, SpindleViewArgs,
};
use anyhow::{anyhow, Result};

pub async fn run(_cli: &Cli, cmd: SpindleCommand) -> Result<()> {
    match cmd {
        SpindleCommand::List(args) => list(args).await,
        SpindleCommand::Runs(args) => runs(args).await,
        SpindleCommand::Config(args) => config(args).await,
        SpindleCommand::Run(args) => run_pipeline(args).await,
        SpindleCommand::Logs(args) => logs(args).await,
        SpindleCommand::Cancel(args) => cancel(args).await,
        SpindleCommand::Status(args) => status(args).await,
        SpindleCommand::View(args) => view(args).await,
        SpindleCommand::Secret(cmd) => secret(cmd).await,
    }
}

/// `list` and `runs` are the same question now. It used to read a
/// sh.tangled.pipeline record collection, which no longer exists: an account
/// that has run hundreds of pipelines has zero such records, and the
/// collection is absent from describeRepo entirely. Pipelines live on the
/// spindle, not on a PDS.
async fn list(args: SpindleListArgs) -> Result<()> {
    runs(crate::cli::SpindleRunsArgs {
        repo: args.repo,
        status: None,
        limit: 20,
    })
    .await
}

async fn runs(args: SpindleRunsArgs) -> Result<()> {
    let (ctx, api) = spindle_context(args.repo.as_deref()).await?;
    let res = api.query_pipelines(&ctx.repo_did, args.limit, None).await?;

    let pipelines: Vec<_> = res
        .pipelines
        .into_iter()
        .filter(|p| match args.status.as_deref() {
            Some(want) => p.workflows.iter().any(|w| w.status == want),
            None => true,
        })
        .collect();

    if pipelines.is_empty() {
        println!("No pipelines found");
        return Ok(());
    }

    let headers = ["PIPELINE", "WORKFLOW", "STATUS", "REF", "COMMIT", "STARTED"];
    let mut rows: Vec<[String; 6]> = Vec::new();
    for p in &pipelines {
        let git_ref = p.trigger_ref().unwrap_or("-").to_string();
        let commit = p.commit.chars().take(10).collect::<String>();
        for w in &p.workflows {
            rows.push([
                p.id.clone(),
                w.name.clone(),
                w.status.clone(),
                git_ref.clone(),
                commit.clone(),
                w.started_at.clone().unwrap_or_else(|| "-".into()),
            ]);
        }
    }
    print_table(&headers, &rows);

    // The status is a summary; the reason a workflow failed is in its error.
    for p in &pipelines {
        for w in &p.workflows {
            if let Some(err) = w.error.as_deref().filter(|e| !e.is_empty()) {
                println!("\n{} {}: {}", p.id, w.name, err);
            }
        }
    }
    Ok(())
}

fn print_table<const N: usize>(headers: &[&str; N], rows: &[[String; N]]) {
    let widths: [usize; N] = std::array::from_fn(|i| {
        headers[i]
            .len()
            .max(rows.iter().map(|r| r[i].len()).max().unwrap_or(0))
    });
    let line = |cells: &[String; N]| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{:<w$}", c, w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
    };
    let head: [String; N] = std::array::from_fn(|i| headers[i].to_string());
    println!("{}", line(&head).trim_end());
    for row in rows {
        println!("{}", line(row).trim_end());
    }
}

/// Everything a spindle call needs: which spindle, and the repo's own DID.
struct SpindleContext {
    session: tangled_config::session::Session,
    pds: String,
    repo_did: String,
    owner: String,
    name: String,
}

/// The branch tip on the knot. Tangled serves git over https, so this needs
/// no ssh key.
fn remote_branch_sha(owner: &str, name: &str, branch: &str) -> Result<String> {
    let url = format!("https://tangled.org/{owner}/{name}");
    let mut remote = git2::Remote::create_detached(url.as_str())?;
    remote.connect(git2::Direction::Fetch)?;
    let want = format!("refs/heads/{branch}");
    let sha = remote
        .list()?
        .iter()
        .find(|h| h.name() == want)
        .map(|h| h.oid().to_string())
        .ok_or_else(|| anyhow!("no {want} on {url}"))?;
    remote.disconnect().ok();
    Ok(sha)
}

async fn spindle_context(
    repo: Option<&str>,
) -> Result<(SpindleContext, tangled_api::TangledClient)> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    // No --repo: infer it from the checkout we are standing in.
    let repo_ref = match repo {
        Some(r) => r.to_string(),
        None => crate::target::repo_from_cwd().unwrap_or_else(|| session.handle.clone()),
    };
    let (owner, name) = parse_repo_ref(&repo_ref, &session.handle);
    let (owner, name) = (owner.to_string(), name.to_string());
    let info = pds_client
        .get_repo_info(&owner, &name, Some(session.access_jwt.as_str()))
        .await?;
    let spindle_base = info
        .spindle
        .clone()
        .or_else(|| std::env::var("TANGLED_SPINDLE_BASE").ok())
        .unwrap_or_else(|| "https://spindle.tangled.sh".to_string());
    // Pipelines are keyed by the repo's own DID, not the owner's.
    let repo_did = info
        .repo_did
        .clone()
        .ok_or_else(|| anyhow!("repo {owner}/{name} has no repoDid; recreate it"))?;
    Ok((
        SpindleContext {
            session,
            pds,
            repo_did,
            owner,
            name,
        },
        crate::util::make_client(&spindle_base),
    ))
}
async fn config(args: SpindleConfigArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;

    if args.enable && args.disable {
        return Err(anyhow!("Cannot use --enable and --disable together"));
    }

    if !args.enable && !args.disable && args.url.is_none() {
        return Err(anyhow!("Must provide --enable, --disable, or --url"));
    }

    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);

    let (owner, name) = parse_repo_ref(
        args.repo.as_deref().unwrap_or(&session.handle),
        &session.handle,
    );
    let info = pds_client
        .get_repo_info(owner, name, Some(session.access_jwt.as_str()))
        .await?;

    let new_spindle = if args.disable {
        None
    } else if let Some(url) = args.url.as_deref() {
        Some(url)
    } else if args.enable {
        // Default spindle URL
        Some("https://spindle.tangled.sh")
    } else {
        return Err(anyhow!("Invalid flags combination"));
    };

    pds_client
        .update_repo_spindle(
            &info.did,
            &info.rkey,
            new_spindle,
            &pds,
            &session.access_jwt,
        )
        .await?;

    if args.disable {
        println!("Disabled spindle for {}/{}", owner, name);
    } else {
        println!(
            "Enabled spindle for {}/{} ({})",
            owner,
            name,
            new_spindle.unwrap_or_default()
        );
    }
    Ok(())
}

async fn run_pipeline(args: SpindleRunArgs) -> Result<()> {
    let (ctx, api) = spindle_context(args.repo.as_deref()).await?;
    let branch = args.branch.as_deref().unwrap_or("main");
    let sha = match args.sha.clone() {
        Some(s) => s,
        None => remote_branch_sha(&ctx.owner, &ctx.name, branch)?,
    };
    let git_ref = format!("refs/heads/{branch}");
    let id = api
        .trigger_pipeline(
            &ctx.pds,
            &ctx.session.access_jwt,
            &ctx.repo_did,
            &sha,
            Some(&git_ref),
            &[],
        )
        .await?;
    println!(
        "Triggered pipeline {id} for {git_ref} at {}",
        &sha[..10.min(sha.len())]
    );
    if args.wait {
        stream_logs(&api, &id, &[]).await?;
    }
    Ok(())
}

async fn logs(args: SpindleLogsArgs) -> Result<()> {
    // The argument is a pipeline id. A pipeline runs several workflows, so an
    // optional --workflow narrows the stream to one of them.
    let (_, api) = spindle_context(args.repo.as_deref()).await?;
    let workflows: Vec<String> = args.workflow.clone().into_iter().collect();
    stream_logs(&api, &args.pipeline, &workflows).await
}

/// Print a pipeline's logs. A finished pipeline replays in full and the
/// spindle then closes the stream, so this terminates on its own.
async fn stream_logs(
    api: &tangled_api::TangledClient,
    pipeline_id: &str,
    workflows: &[String],
) -> Result<()> {
    use tangled_api::ci_logs::{subscribe_pipeline_logs, LogEvent};

    let url = api.pipeline_logs_url(pipeline_id, workflows)?;
    let mut current = String::new();
    subscribe_pipeline_logs(&url, |event| {
        match event {
            LogEvent::Control(c) => {
                if c.workflow != current {
                    println!("\n=== {} ===", c.workflow);
                    current = c.workflow.clone();
                }
                match c.command.as_deref() {
                    Some(cmd) => println!("--- step {} [{}] $ {}", c.step, c.kind, cmd),
                    None if !c.status.is_empty() => {
                        println!("--- step {} [{}] {}", c.step, c.kind, c.status)
                    }
                    None => println!("--- step {} [{}]", c.step, c.kind),
                }
            }
            LogEvent::Data(d) => {
                if d.workflow != current {
                    println!("\n=== {} ===", d.workflow);
                    current = d.workflow.clone();
                }
                print!("{}", d.content);
                if !d.content.ends_with('\n') {
                    println!();
                }
            }
        }
        Ok(())
    })
    .await
}
async fn secret(cmd: SpindleSecretCommand) -> Result<()> {
    match cmd {
        SpindleSecretCommand::List(args) => secret_list(args).await,
        SpindleSecretCommand::Add(args) => secret_add(args).await,
        SpindleSecretCommand::Remove(args) => secret_remove(args).await,
    }
}

async fn secret_list(args: SpindleSecretListArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let info = pds_client
        .get_repo_info(owner, name, Some(session.access_jwt.as_str()))
        .await?;
    let repo_at = format!("at://{}/sh.tangled.repo/{}", info.did, info.rkey);

    // Get spindle base from repo config or use default
    let spindle_base = info
        .spindle
        .clone()
        .or_else(|| std::env::var("TANGLED_SPINDLE_BASE").ok())
        .unwrap_or_else(|| "https://spindle.tangled.sh".to_string());
    let api = crate::util::make_client(&spindle_base);

    let secrets = api
        .list_repo_secrets(&pds, &session.access_jwt, &repo_at)
        .await?;
    if secrets.is_empty() {
        println!("No secrets configured for {}", args.repo);
    } else {
        println!("KEY\tCREATED AT\tCREATED BY");
        for s in secrets {
            println!("{}\t{}\t{}", s.key, s.created_at, s.created_by);
        }
    }
    Ok(())
}

async fn secret_add(args: SpindleSecretAddArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let info = pds_client
        .get_repo_info(owner, name, Some(session.access_jwt.as_str()))
        .await?;
    let repo_at = format!("at://{}/sh.tangled.repo/{}", info.did, info.rkey);

    // Get spindle base from repo config or use default
    let spindle_base = info
        .spindle
        .clone()
        .or_else(|| std::env::var("TANGLED_SPINDLE_BASE").ok())
        .unwrap_or_else(|| "https://spindle.tangled.sh".to_string());
    let api = crate::util::make_client(&spindle_base);

    // Handle special value patterns: @file or - (stdin)
    let value = if args.value == "-" {
        // Read from stdin
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else if let Some(path) = args.value.strip_prefix('@') {
        // Read from file, expand ~ if needed
        let expanded_path = if path.starts_with("~/") {
            if let Ok(home) = std::env::var("HOME") {
                path.replacen("~/", &format!("{}/", home), 1)
            } else {
                path.to_string()
            }
        } else {
            path.to_string()
        };
        std::fs::read_to_string(&expanded_path)
            .map_err(|e| anyhow!("Failed to read file '{}': {}", expanded_path, e))?
    } else {
        // Use value as-is
        args.value
    };

    api.add_repo_secret(&pds, &session.access_jwt, &repo_at, &args.key, &value)
        .await?;
    println!("Added secret '{}' to {}", args.key, args.repo);
    Ok(())
}

async fn secret_remove(args: SpindleSecretRemoveArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let pds = session
        .pds
        .clone()
        .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
        .unwrap_or_else(|| "https://bsky.social".into());
    let pds_client = crate::util::make_client(&pds);
    let (owner, name) = parse_repo_ref(&args.repo, &session.handle);
    let info = pds_client
        .get_repo_info(owner, name, Some(session.access_jwt.as_str()))
        .await?;
    let repo_at = format!("at://{}/sh.tangled.repo/{}", info.did, info.rkey);

    // Get spindle base from repo config or use default
    let spindle_base = info
        .spindle
        .clone()
        .or_else(|| std::env::var("TANGLED_SPINDLE_BASE").ok())
        .unwrap_or_else(|| "https://spindle.tangled.sh".to_string());
    let api = crate::util::make_client(&spindle_base);

    api.remove_repo_secret(&pds, &session.access_jwt, &repo_at, &args.key)
        .await?;
    println!("Removed secret '{}' from {}", args.key, args.repo);
    Ok(())
}

fn parse_repo_ref<'a>(spec: &'a str, default_owner: &'a str) -> (&'a str, &'a str) {
    if let Some((owner, name)) = spec.split_once('/') {
        (owner, name)
    } else {
        (default_owner, spec)
    }
}

async fn cancel(args: SpindleCancelArgs) -> Result<()> {
    let (ctx, api) = spindle_context(args.repo.as_deref()).await?;
    api.cancel_pipeline(
        &ctx.pds,
        &ctx.session.access_jwt,
        &args.pipeline,
        &ctx.repo_did,
        &args.workflow,
    )
    .await?;
    if args.workflow.is_empty() {
        println!("Cancelled pipeline {}", args.pipeline);
    } else {
        println!(
            "Cancelled {} in pipeline {}",
            args.workflow.join(", "),
            args.pipeline
        );
    }
    Ok(())
}

async fn status(args: SpindleStatusArgs) -> Result<()> {
    let (ctx, api) = spindle_context(args.repo.as_deref()).await?;
    let res = api.query_pipelines(&ctx.repo_did, 1, None).await?;
    let Some(pipeline) = res.pipelines.first() else {
        println!("No pipelines yet");
        return Ok(());
    };
    render_pipeline(pipeline);
    Ok(())
}

async fn view(args: SpindleViewArgs) -> Result<()> {
    let (_, api) = spindle_context(args.repo.as_deref()).await?;
    let pipeline = api.get_pipeline(&args.pipeline).await?;
    render_pipeline(&pipeline);
    Ok(())
}

fn render_pipeline(p: &tangled_api::ci::Pipeline) {
    println!("PIPELINE:  {}", p.id);
    println!("COMMIT:    {}", p.commit);
    if let Some(git_ref) = p.trigger_ref() {
        println!("REF:       {git_ref}");
    }
    if let Some(created) = &p.created_at {
        println!("CREATED:   {created}");
    }
    println!();
    if p.workflows.is_empty() {
        println!("No workflows");
        return;
    }
    let headers = ["WORKFLOW", "STATUS", "STARTED", "FINISHED"];
    let rows: Vec<[String; 4]> = p
        .workflows
        .iter()
        .map(|w| {
            [
                w.name.clone(),
                w.status.clone(),
                w.started_at.clone().unwrap_or_else(|| "-".into()),
                w.finished_at.clone().unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    print_table(&headers, &rows);
    // A status alone does not say why something failed; the error does.
    for w in &p.workflows {
        if let Some(err) = w.error.as_deref().filter(|e| !e.is_empty()) {
            println!("\n{}: {}", w.name, err);
        }
    }
}
