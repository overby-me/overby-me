use crate::cli::{ApiCommand, ApiGetArgs, ApiPostArgs, Cli};
use anyhow::{anyhow, Result};

pub async fn run(_cli: &Cli, cmd: ApiCommand) -> Result<()> {
    match cmd {
        ApiCommand::Get(args) => get(args).await,
        ApiCommand::Post(args) => post(args).await,
    }
}

fn parse_params(raw: &[String]) -> Result<Vec<(&str, String)>> {
    raw.iter()
        .map(|p| {
            let (k, v) = p
                .split_once('=')
                .ok_or_else(|| anyhow!("invalid param '{}', expected key=value", p))?;
            Ok((k, v.to_string()))
        })
        .collect()
}

async fn get(args: ApiGetArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let base = if args.pds {
        session
            .pds
            .clone()
            .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
            .unwrap_or_else(|| "https://bsky.social".into())
    } else {
        std::env::var("TANGLED_API_BASE").unwrap_or_else(|_| "https://tngl.sh".into())
    };
    let client = crate::util::make_client(&base);
    let params = parse_params(&args.params)?;
    let res: serde_json::Value = client
        .get_json(&args.method, &params, Some(&session.access_jwt))
        .await?;
    println!("{}", serde_json::to_string_pretty(&res)?);
    Ok(())
}

async fn post(args: ApiPostArgs) -> Result<()> {
    let session = crate::util::load_session_with_refresh().await?;
    let base = if args.pds {
        session
            .pds
            .clone()
            .or_else(|| std::env::var("TANGLED_PDS_BASE").ok())
            .unwrap_or_else(|| "https://bsky.social".into())
    } else {
        std::env::var("TANGLED_API_BASE").unwrap_or_else(|_| "https://tngl.sh".into())
    };
    let client = crate::util::make_client(&base);

    let body: serde_json::Value = match args.input.as_deref() {
        Some("-") => {
            let stdin = std::io::read_to_string(std::io::stdin())?;
            serde_json::from_str(&stdin)?
        }
        Some(path) => {
            let content = std::fs::read_to_string(path)?;
            serde_json::from_str(&content)?
        }
        None => {
            // Build JSON from --param key=value pairs
            let mut map = serde_json::Map::new();
            for p in &args.params {
                let (k, v) = p
                    .split_once('=')
                    .ok_or_else(|| anyhow!("invalid param '{}', expected key=value", p))?;
                map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
            }
            serde_json::Value::Object(map)
        }
    };

    let res: serde_json::Value = client
        .post_json_pub(&args.method, &body, Some(&session.access_jwt))
        .await?;
    println!("{}", serde_json::to_string_pretty(&res)?);
    Ok(())
}
