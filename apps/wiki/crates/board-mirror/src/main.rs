//! `board-mirror follow --pds <url> --repo <did> --dir <dir> [--every <secs>]`
//! keeps an independent copy of a published ballot board;
//! `board-mirror check --dir <dir> [--key <did:key>]` counts it. Both exit
//! non-zero when the custodian's word does not hold up.

use std::path::PathBuf;

fn flag(args: &[String], name: &str) -> Option<String> {
    let at = args.iter().position(|a| a == name)?;
    args.get(at + 1).cloned()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = flag(&args, "--dir").map(PathBuf::from);
    match (args.first().map(String::as_str), dir) {
        (Some("follow"), Some(dir)) => {
            let (Some(pds), Some(repo)) = (flag(&args, "--pds"), flag(&args, "--repo")) else {
                return usage();
            };
            let every = flag(&args, "--every").and_then(|s| s.parse::<u64>().ok());
            loop {
                match board_mirror::follow_once(&pds, &repo, &dir).await {
                    Ok(seen) => {
                        println!("{} new, {} alarms", seen.new, seen.alarms.len());
                        for alarm in &seen.alarms {
                            eprintln!("ALARM {}: {}", alarm.uri, alarm.what);
                        }
                        if every.is_none() {
                            std::process::exit(i32::from(!seen.alarms.is_empty()));
                        }
                    }
                    Err(e) => {
                        eprintln!("could not read the repo: {e}");
                        if every.is_none() {
                            std::process::exit(2);
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(every.unwrap_or(60))).await;
            }
        }
        (Some("check"), Some(dir)) => {
            let key = flag(&args, "--key");
            let mut failed = false;
            for poll in board_mirror::check(&dir, key.as_deref()) {
                match (&poll.counts, poll.problems.is_empty()) {
                    (None, true) => println!("open      {} ({})", poll.question, poll.poll),
                    (Some(counts), true) => {
                        println!("confirmed {} {counts:?} ({})", poll.question, poll.poll);
                    }
                    _ => {
                        failed = true;
                        println!("DISPUTED  {} ({})", poll.question, poll.poll);
                        for problem in &poll.problems {
                            println!("          {problem}");
                        }
                    }
                }
            }
            std::process::exit(i32::from(failed));
        }
        _ => usage(),
    }
}

fn usage() {
    eprintln!(
        "usage: board-mirror follow --pds <url> --repo <did> --dir <dir> [--every <secs>]\n       \
         board-mirror check --dir <dir> [--key <did:key>]"
    );
    std::process::exit(2);
}
