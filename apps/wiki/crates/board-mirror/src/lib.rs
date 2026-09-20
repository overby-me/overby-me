//! An independent copy of a published ballot board, and the check of it.
//!
//! The organization publishes every board and could quietly unpublish a ballot
//! or rewrite one. A record in a repo is not immutable: what makes a deletion
//! evidence and not a memory hole is somebody else having kept a copy. This is
//! that somebody's tool. It reads the board account's public repo, needs no
//! account and no word from the AppView, keeps what it sees in a file it only
//! ever appends to, and says so when something it has seen is gone or changed.
//!
//! A board that went into a group's atproto space is not public, and is read
//! the way a member's application reads the space: [`follow_space_once`], by an
//! app password of the member's own account, holding the organization's repo to
//! the commit the organization signed.
//!
//! [`check`] then counts each closed poll for itself, from its own copy: the
//! ballots' signatures under the poll's published key, the first of a repeated
//! token, the rules, the digest of the board, and the custodian's signature on
//! the close-out that claims them.

use ballot_spec::custody::{CloseOut, board_digest, verify};
use ballot_spec::provisional::{ProvisionalEntry, decode_bytes};
use ballot_spec::{BallotRules, BoardEntry, IssuerPublicKey, MessageRandomizer, Signature};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

pub const POLL: &str = "wiki.radikal.poll";
pub const ENTRY: &str = "wiki.radikal.ballotEntry";
pub const CLOSEOUT: &str = "wiki.radikal.pollCloseOut";

type Failure = Box<dyn std::error::Error + Send + Sync>;

/// One record as it was first seen, or seen changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub seen_at: String,
    pub uri: String,
    pub cid: String,
    pub value: serde_json::Value,
}

/// Something a custodian is not supposed to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alarm {
    pub at: String,
    pub uri: String,
    pub what: String,
}

/// Whether `uri` is a record of `collection`. By the whole segment: one of
/// these names begins with another. A record in a space has the space and its
/// author before its collection.
fn is_a(uri: &str, collection: &str) -> bool {
    let mut segments = uri.split('/').skip(3);
    match segments.next() {
        Some("space") => segments.nth(3) == Some(collection),
        first => first == Some(collection),
    }
}

fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("unix:{secs}")
}

fn read_lines<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn append<T: Serialize>(path: &Path, line: &T) -> Result<(), Failure> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(line)?)?;
    Ok(())
}

async fn list(pds: &str, repo: &str, collection: &str) -> Result<Vec<Seen>, Failure> {
    let client = reqwest::Client::new();
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut query = vec![
            ("repo", repo.to_string()),
            ("collection", collection.to_string()),
            ("limit", "100".to_string()),
        ];
        if let Some(cursor) = &cursor {
            query.push(("cursor", cursor.clone()));
        }
        let url = format!(
            "{}/xrpc/com.atproto.repo.listRecords",
            pds.trim_end_matches('/')
        );
        let page: serde_json::Value = client
            .get(url)
            .query(&query)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let records = page["records"].as_array().cloned().unwrap_or_default();
        for record in &records {
            out.push(Seen {
                seen_at: now(),
                uri: record["uri"].as_str().unwrap_or_default().to_string(),
                cid: record["cid"].as_str().unwrap_or_default().to_string(),
                value: record["value"].clone(),
            });
        }
        cursor = page["cursor"].as_str().map(str::to_string);
        if records.is_empty() || cursor.is_none() {
            return Ok(out);
        }
    }
}

/// What one look at the repo found.
#[derive(Debug, Default, PartialEq)]
pub struct Followed {
    pub new: usize,
    pub alarms: Vec<Alarm>,
}

/// Whether a poll's announcement changed only in the one way it may: closing.
fn only_closed(before: &serde_json::Value, after: &serde_json::Value) -> bool {
    let (mut was, mut is) = (before.clone(), after.clone());
    for version in [&mut was, &mut is] {
        if let Some(fields) = version.as_object_mut() {
            fields.remove("state");
        }
    }
    was == is && before["state"] == "open" && after["state"] == "closed"
}

/// Look at the board account's repo once: keep what is new, and raise an alarm
/// for anything seen before that is gone or says something else.
pub async fn follow_once(pds: &str, repo: &str, dir: &Path) -> Result<Followed, Failure> {
    let mut there = Vec::new();
    for collection in [POLL, ENTRY, CLOSEOUT] {
        there.extend(list(pds, repo, collection).await?);
    }
    keep(there, dir)
}

/// A member of the group whose board is mirrored, as their own PDS knows them.
pub struct Member {
    pub pds: String,
    /// A handle or a DID.
    pub identifier: String,
    /// An app password: all it is used for is to be let into the space.
    pub password: String,
    /// Which application this mirror is, where the organization's spaces admit
    /// applications by a list: a `client_id` the organization has named, and
    /// the key its client metadata publishes.
    pub client: Option<(String, atproto_spaces::attestation::ClientKey)>,
}

/// [`follow_once`], of the board in a group's space
/// (`at://{organization}/space/{type}/{key}`), read as `member`. `directory` is
/// where a `did:plc` resolves. The organization's repo is listed whole and held
/// to the commit the organization signed, so a host cannot serve one member a
/// board of its own making.
pub async fn follow_space_once(
    member: &Member,
    space: &str,
    directory: &str,
    dir: &Path,
) -> Result<Followed, Failure> {
    use atproto_spaces::client::Host;
    let organization = space
        .strip_prefix("at://")
        .and_then(|rest| rest.split('/').next())
        .ok_or("a space is at://{organization}/space/{type}/{key}")?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let own = Host::new(http.clone(), &member.pds);
    let (_, session) = own
        .create_session(&member.identifier, &member.password)
        .await?;
    let authority = atproto_spaces::directory::Directory::new(http.clone(), directory)
        .resolve(organization)
        .await?;
    let host = authority.space_host().or(authority.pds());
    let host = Host::new(http, host.ok_or("the organization names no host")?);
    let attestation = member
        .client
        .as_ref()
        .map(|(client_id, key)| key.attest(client_id, organization));
    let credential = atproto_spaces::credential::Credential::obtain(
        &own,
        &session,
        &host,
        space,
        attestation.as_deref(),
    )
    .await?;
    let pulled = atproto_spaces::sync::pull(
        &host,
        credential.auth(),
        space,
        organization,
        &authority.signing_key,
        &mut atproto_spaces::sync::Copy::default(),
    )
    .await?;
    let atproto_spaces::sync::Pulled::Everything(records) = pulled else {
        return Err("a first pull was not the whole repo".into());
    };
    let there = records
        .into_iter()
        .filter(|r| [POLL, ENTRY, CLOSEOUT].contains(&r.collection.as_str()))
        .map(|r| Seen {
            seen_at: now(),
            uri: format!("{space}/{organization}/{}/{}", r.collection, r.rkey),
            cid: r.cid,
            value: r.value,
        })
        .collect();
    keep(there, dir)
}

/// Hold what is `there` now against the copy in `dir`, and add to the copy.
fn keep(there: Vec<Seen>, dir: &Path) -> Result<Followed, Failure> {
    std::fs::create_dir_all(dir)?;
    let (board, alarms) = (dir.join("board.jsonl"), dir.join("alarms.jsonl"));
    let mut known: BTreeMap<String, Seen> = BTreeMap::new();
    for seen in read_lines::<Seen>(&board) {
        known.insert(seen.uri.clone(), seen);
    }
    let mut out = Followed::default();
    let mut raise = |uri: &str, what: String| -> Result<(), Failure> {
        let alarm = Alarm {
            at: now(),
            uri: uri.to_string(),
            what,
        };
        append(&alarms, &alarm)?;
        out.alarms.push(alarm);
        Ok(())
    };
    for seen in &there {
        match known.get(&seen.uri) {
            None => {
                append(&board, seen)?;
                out.new += 1;
            }
            Some(before) if before.cid == seen.cid => {}
            Some(before) if is_a(&seen.uri, POLL) && only_closed(&before.value, &seen.value) => {
                append(&board, seen)?;
            }
            Some(before) => {
                raise(
                    &seen.uri,
                    format!("rewritten: was {}, is {}", before.cid, seen.cid),
                )?;
                append(&board, seen)?;
            }
        }
    }
    let uris: std::collections::BTreeSet<&str> = there.iter().map(|s| s.uri.as_str()).collect();
    let raised: std::collections::BTreeSet<String> = read_lines::<Alarm>(&alarms)
        .into_iter()
        .filter(|a| a.what == "gone")
        .map(|a| a.uri)
        .collect();
    for uri in known.keys() {
        if !uris.contains(uri.as_str()) && !raised.contains(uri) {
            raise(uri, "gone".to_string())?;
        }
    }
    Ok(out)
}

/// What the mirror's own count of one poll came to.
#[derive(Debug, Clone, PartialEq)]
pub struct PollCheck {
    pub poll: String,
    pub question: String,
    /// Empty when everything the custodian said holds up.
    pub problems: Vec<String>,
    /// The mirror's own count, once the poll has closed.
    pub counts: Option<Vec<u64>>,
}

fn entry_of(value: &serde_json::Value) -> Option<(ProvisionalEntry, BoardEntry)> {
    let provisional = ProvisionalEntry {
        token: value["token"].as_str()?.to_string(),
        msg_randomizer: value["msgRandomizer"].as_str().map(str::to_string),
        signature: value["signature"].as_str()?.to_string(),
        choices: serde_json::from_value(value["choices"].clone()).ok()?,
    };
    let (token, randomizer, signature) = decode_bytes(&provisional).ok()?;
    let msg_randomizer = match randomizer {
        Some(bytes) => Some(MessageRandomizer(bytes.try_into().ok()?)),
        None => None,
    };
    let entry = BoardEntry {
        token,
        msg_randomizer,
        signature: Signature(signature),
        choices: provisional.choices.clone(),
    };
    Some((provisional, entry))
}

/// Count every closed poll in the mirror's copy, and hold each close-out to it.
/// `trusted_key` is the custody key as the checker learned it some other way;
/// without one, each poll's announcement is taken at its word for it.
pub fn check(dir: &Path, trusted_key: Option<&str>) -> Vec<PollCheck> {
    let seen: Vec<Seen> = read_lines(&dir.join("board.jsonl"));
    // The first version of an announcement is the one ballots were cast under.
    let mut polls: BTreeMap<String, &Seen> = BTreeMap::new();
    for record in seen.iter().filter(|s| is_a(&s.uri, POLL)) {
        polls.entry(record.uri.clone()).or_insert(record);
    }
    let mut out = Vec::new();
    for (uri, poll) in polls {
        let mut problems = Vec::new();
        let announced = &poll.value;
        let mut entries: BTreeMap<String, &Seen> = BTreeMap::new();
        for entry in seen.iter().filter(|s| is_a(&s.uri, ENTRY)) {
            if entry.value["pollRef"]["uri"] == uri.as_str() {
                entries.entry(entry.uri.clone()).or_insert(entry);
            }
        }
        let close = seen
            .iter()
            .find(|s| is_a(&s.uri, CLOSEOUT) && s.value["poll"] == uri.as_str());
        let Some(close) = close else {
            out.push(PollCheck {
                poll: uri.clone(),
                question: announced["question"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                problems,
                counts: None,
            });
            continue;
        };
        let said = &close.value;
        let text = |key: &str| said[key].as_str().unwrap_or_default().to_string();
        let natural = |v: &serde_json::Value| v.as_u64().unwrap_or(0);

        let key = announced["custodyKey"].as_str().unwrap_or_default();
        if trusted_key.is_some_and(|trusted| trusted != key) {
            problems.push(format!("announced under {key}, which is not the key given"));
        }
        if text("key") != key {
            problems
                .push("the close-out is signed by another key than the announcement names".into());
        }
        let claimed = CloseOut {
            poll: text("pollId"),
            entries: natural(&said["entries"]),
            issued: natural(&said["issued"]),
            counts: serde_json::from_value(said["counts"].clone()).unwrap_or_default(),
            board_digest: text("boardDigest"),
            closed_at: text("closedAt"),
        };
        if !verify(key, &claimed.payload(), &text("sig")) {
            problems.push("the close-out's signature does not verify".into());
        }

        let mut provisional = Vec::new();
        let mut ballots = Vec::new();
        for (entry_uri, entry) in &entries {
            match entry_of(&entry.value) {
                Some((as_published, ballot)) => {
                    provisional.push(as_published);
                    ballots.push(ballot);
                }
                None => problems.push(format!("{entry_uri} is not a readable ballot")),
            }
        }
        if board_digest(&provisional) != claimed.board_digest {
            problems.push(format!(
                "the board here ({} ballots) is not the board that was signed for ({})",
                provisional.len(),
                claimed.entries
            ));
        }
        if claimed.entries > claimed.issued {
            problems.push(format!(
                "{} ballots on the board and {} tokens issued",
                claimed.entries, claimed.issued
            ));
        }
        let rules = BallotRules {
            options: announced["options"].as_array().map_or(0, Vec::len),
            min: natural(&announced["minVote"]) as usize,
            max: natural(&announced["maxVote"]) as usize,
            blank: announced["blank"] == true,
        };
        let issuer = announced["issuerPubkey"]
            .as_str()
            .and_then(|b64| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(b64)
                    .ok()
            })
            .and_then(|der| IssuerPublicKey::from_der(&der).ok());
        let counts = match issuer {
            Some(issuer) => {
                let recount = ballot_spec::recount(&issuer, &rules, ballots);
                for (at, why) in &recount.dropped {
                    problems.push(format!("ballot {at} does not count: {why:?}"));
                }
                if recount.counts != claimed.counts {
                    problems.push(format!(
                        "counted {:?}, and {:?} was announced",
                        recount.counts, claimed.counts
                    ));
                }
                Some(recount.counts)
            }
            None => {
                problems.push("the announcement carries no readable issuer key".into());
                None
            }
        };
        out.push(PollCheck {
            poll: uri,
            question: announced["question"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            problems,
            counts,
        });
    }
    out
}
