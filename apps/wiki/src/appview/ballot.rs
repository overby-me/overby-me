//! A secret ballot, from the voter's side.
//!
//! The voter blinds a random token, the AppView signs it unseen, and the
//! unblinded signature is spent on a ballot sent with NO session: the AppView
//! knows who was given a token and what each token voted, and never both.
//!
//! It signs a voter's tokens once, and again only for the same blinded tokens.
//! So what was blinded is kept BEFORE it is sent: a reply lost on the way is
//! asked for again, where a fresh request would be refused and the vote lost.

use super::vote::read_poll;
use super::{ask_quiet, client, reported, said};
use appview_client::{cast_ballot, get_board_entry, issue_ballot_tokens, Error};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};

/// What the poll screen takes to mean "this voter's ballot is in".
const ALREADY_VOTED: &str = "already voted";

#[derive(Serialize, Deserialize, Default)]
struct Kept {
    /// One per unit of the voter's weight, until it has landed.
    tokens: Vec<KeptToken>,
    /// Every ballot has landed.
    cast: bool,
    /// What is kept of each ballot that landed: the only proof that an entry on
    /// the board is this voter's, which nobody can make again
    /// (`docs/ballot-verify-ux.md`). Guarded as the session is, and gone with
    /// it at sign-out: it says how its owner voted.
    #[serde(default)]
    stubs: Vec<Stub>,
}

/// One landed ballot, as its voter keeps it.
#[derive(Serialize, Deserialize, Clone)]
struct Stub {
    token: String,
    choices: Vec<i64>,
    position: i64,
    /// The custodian's signed receipt, whole, as evidence for a dispute.
    receipt: serde_json::Value,
}

fn stub_of(receipt: &appview_client::defs::ReceiptView) -> Stub {
    Stub {
        token: receipt.token.clone(),
        choices: receipt.choices.clone(),
        position: receipt.position,
        receipt: serde_json::to_value(receipt).unwrap_or_default(),
    }
}

// No `Debug`: a `{:?}` of this in a log line would be the vote's secret.
#[derive(Serialize, Deserialize)]
struct KeptToken {
    nullifier: String,
    blind_message: String,
    secret: String,
    msg_randomizer: Option<String>,
    /// Unblinded, once the AppView has signed.
    signature: Option<String>,
    landed: bool,
}

thread_local! {
    /// Where this is kept where there is no browser storage: under `cargo
    /// test`, and in a browser that refuses it, for as long as the tab lives.
    static MEMORY: std::cell::RefCell<std::collections::HashMap<String, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Per voter as well as per poll: two people may vote from one browser.
fn key(did: &str, poll: &str) -> String {
    format!("wiki.ballot.{did}.{poll}")
}

fn kept(key: &str) -> Kept {
    #[cfg(target_arch = "wasm32")]
    let stored = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten());
    #[cfg(not(target_arch = "wasm32"))]
    let stored = None;
    stored
        .or_else(|| MEMORY.with(|memory| memory.borrow().get(key).cloned()))
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn keep(key: &str, kept: &Kept) {
    let Ok(json) = serde_json::to_string(kept) else {
        return;
    };
    #[cfg(target_arch = "wasm32")]
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, &json);
    }
    MEMORY.with(|memory| memory.borrow_mut().insert(key.to_string(), json));
}

fn unb64(text: &str) -> Result<Vec<u8>, String> {
    B64.decode(text)
        .map_err(|_| "a kept token is not base64url".to_string())
}

fn crypto(e: ballot_spec::CryptoError) -> String {
    format!("the ballot's signature did not work out: {e}")
}

/// Fresh tokens to have signed, one per ballot the voter holds.
fn blind(key: &ballot_spec::IssuerPublicKey, weight: i64) -> Result<Vec<KeptToken>, String> {
    (0..weight)
        .map(|_| {
            let request = ballot_spec::request_token(key).map_err(crypto)?;
            Ok(KeptToken {
                nullifier: B64.encode(&request.nullifier),
                blind_message: B64.encode(&request.blinding.blind_message.0),
                secret: B64.encode(&request.blinding.secret.0),
                msg_randomizer: request.blinding.msg_randomizer.map(|r| B64.encode(r.0)),
                signature: None,
                landed: false,
            })
        })
        .collect()
}

/// Unblind what the AppView signed. This also checks it against the poll's
/// key, so a signature that would not count is caught here, not on the board.
fn unblind(
    key: &ballot_spec::IssuerPublicKey,
    token: &KeptToken,
    blind_signature: &str,
) -> Result<String, String> {
    let msg_randomizer = match &token.msg_randomizer {
        Some(r) => Some(ballot_spec::MessageRandomizer(
            unb64(r)?
                .try_into()
                .map_err(|_| "a kept randomizer is not 32 bytes".to_string())?,
        )),
        None => None,
    };
    let request = ballot_spec::TokenRequest {
        nullifier: unb64(&token.nullifier)?,
        blinding: ballot_spec::BlindingResult {
            blind_message: ballot_spec::BlindMessage(unb64(&token.blind_message)?),
            secret: ballot_spec::Secret(unb64(&token.secret)?),
            msg_randomizer,
        },
    };
    let signed = ballot_spec::BlindSignature(unb64(blind_signature)?);
    ballot_spec::finalize_token(key, &request, &signed)
        .map(|signature| B64.encode(&signature.0))
        .map_err(crypto)
}

/// Cast a secret ballot, once per ballot the voter holds, all for `choices`.
/// `Err("already voted")` when this voter's ballot is already in.
pub async fn vote_cast_secret(
    token: &str,
    poll: &str,
    _context: Option<&str>,
    choices: &[usize],
) -> Result<(), String> {
    let what = "castBallot";
    let me = super::whoami(token).await.ok_or("not signed in")?;
    let key = key(&me, poll);
    let mut state = kept(&key);
    if state.cast {
        return Err(ALREADY_VOTED.to_string());
    }
    let view = read_poll(Some(token), poll).await.ok_or("no such poll")?;
    let issuer = view
        .issuer_pubkey
        .as_deref()
        .ok_or("that poll is not a secret one")?;
    let issuer = ballot_spec::IssuerPublicKey::from_der(&unb64(issuer)?).map_err(crypto)?;

    if state.tokens.is_empty() {
        let weight = view.viewer.as_ref().map_or(0, |viewer| viewer.weight);
        if weight < 1 {
            return Err("you hold no vote in this poll".to_string());
        }
        state.tokens = blind(&issuer, weight)?;
        keep(&key, &state);
    }
    if state.tokens.iter().any(|t| t.signature.is_none()) {
        let asked = issue_ballot_tokens::Input {
            poll: poll.to_string(),
            blinded: state
                .tokens
                .iter()
                .map(|t| t.blind_message.clone())
                .collect(),
        };
        let mine = client(Some(token));
        let issued = ask_quiet(false, || mine.issue_ballot_tokens(&asked))
            .await
            .map_err(|error| match error.name() {
                // Collected from another browser, which is where they can be cast.
                Some("AlreadyIssued") => said(&error),
                _ => reported(what, &error),
            })?;
        for (kept, signed) in state.tokens.iter_mut().zip(&issued.signatures) {
            kept.signature = Some(unblind(&issuer, kept, signed)?);
        }
        keep(&key, &state);
        #[cfg(test)]
        keep(&format!("{key}.signed"), &state);
    }

    // As nobody: a session on these calls would put the voter's name on them.
    let nobody = client(None);
    let choices: Vec<i64> = choices
        .iter()
        .filter_map(|choice| i64::try_from(*choice).ok())
        .collect();
    let mut stands = true;
    for index in 0..state.tokens.len() {
        let kept_token = &state.tokens[index];
        if kept_token.landed {
            continue;
        }
        let ballot = cast_ballot::Input {
            poll: poll.to_string(),
            token: kept_token.nullifier.clone(),
            msg_randomizer: kept_token.msg_randomizer.clone(),
            signature: kept_token.signature.clone().unwrap_or_default(),
            choices: choices.clone(),
        };
        match ask_quiet(false, || nobody.cast_ballot(&ballot)).await {
            Ok(landed) => state.stubs.push(stub_of(&landed.receipt)),
            // Spent by an earlier attempt whose answer never arrived. What THAT
            // one said is what counts, so ask the board rather than assume.
            Err(Error::Api { error, .. }) if error == "AlreadySpent" => {
                let mine = get_board_entry::Params {
                    poll: poll.to_string(),
                    token: ballot.token.clone(),
                };
                let landed = ask_quiet(true, || nobody.get_board_entry(&mine))
                    .await
                    .map_err(|error| reported(what, &error))?;
                stands &= landed.entry.choices == choices;
                state.stubs.push(stub_of(&landed.receipt));
            }
            Err(error) => return Err(reported(what, &error)),
        }
        state.tokens[index].landed = true;
        keep(&key, &state);
    }
    // The blinding secrets have done their work; the stubs are what is kept.
    keep(
        &key,
        &Kept {
            tokens: Vec::new(),
            cast: true,
            stubs: state.stubs,
        },
    );
    match stands {
        true => Ok(()),
        false => Err(
            "an earlier try had already landed with another choice, and that one counts"
                .to_string(),
        ),
    }
}

/// Whether the caller's secret ballot is in, as far as can be known: this
/// browser cast it, or the tokens went to another one, which then holds the vote.
pub async fn vote_status(token: &str, poll: &str) -> bool {
    let Some(me) = super::whoami(token).await else {
        return false;
    };
    let state = kept(&key(&me, poll));
    if state.cast || !state.tokens.is_empty() {
        return state.cast;
    }
    read_poll(Some(token), poll)
        .await
        .and_then(|view| view.viewer)
        .is_some_and(|viewer| viewer.issued)
}

/// Put a voter back to where their ballot had landed and the answer had not:
/// the tokens still kept, signed, and not known to be spent.
#[cfg(test)]
pub(crate) fn forget_the_cast(did: &str, poll: &str) {
    let key = key(did, poll);
    let earlier = MEMORY.with(|memory| memory.borrow().get(&format!("{key}.signed")).cloned());
    if let Some(json) = earlier {
        MEMORY.with(|memory| memory.borrow_mut().insert(key, json));
    }
}

/// How each ballot this device cast in `poll` stands on the board. Asked as
/// nobody, by the token alone: asking as oneself would pair the two.
pub async fn my_ballots(token: &str, poll: &str) -> Vec<crate::model::BallotStanding> {
    use crate::model::BallotStanding;
    let Some(me) = super::whoami(token).await else {
        return Vec::new();
    };
    let nobody = client(None);
    let mut out = Vec::new();
    for stub in kept(&key(&me, poll)).stubs {
        let mine = get_board_entry::Params {
            poll: poll.to_string(),
            token: stub.token.clone(),
        };
        out.push(
            match ask_quiet(true, || nobody.get_board_entry(&mine)).await {
                Ok(found) if found.entry.choices == stub.choices => BallotStanding::Counted {
                    position: u64::try_from(found.position).unwrap_or(0),
                },
                Ok(_) => BallotStanding::RecordedDifferently,
                Err(error) if super::is_absent(&error) => BallotStanding::NotOnTheBoard,
                // No answer is not an answer: say nothing of this one.
                Err(_) => continue,
            },
        );
    }
    out
}

/// Forget every stub on this device. With the session, at sign-out.
pub(crate) fn forget_all() {
    MEMORY.with(|memory| memory.borrow_mut().clear());
    #[cfg(target_arch = "wasm32")]
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let ours: Vec<String> = (0..storage.length().unwrap_or(0))
            .filter_map(|i| storage.key(i).ok().flatten())
            .filter(|k| k.starts_with("wiki.ballot."))
            .collect();
        for key in ours {
            let _ = storage.remove_item(&key);
        }
    }
}

/// Count a closed poll's board again, here: every ballot's signature under the
/// poll's key, the first of a repeated token, the rules, and then the count,
/// the board's digest and the custodian's signature on the close-out. The code
/// that counts is `ballot_spec`, which is what the AppView counted with.
pub async fn recount_poll(
    access_token: Option<&str>,
    poll: &str,
) -> Result<crate::model::Recounted, String> {
    use ballot_spec::custody::{board_digest, verify, CloseOut};
    use ballot_spec::provisional::{decode_bytes, ProvisionalEntry};
    let view = read_poll(access_token, poll).await.ok_or("no such poll")?;
    let client = client(access_token);
    let params = appview_client::get_board::Params {
        poll: poll.to_string(),
    };
    let board = super::ask("getBoard", true, || client.get_board(&params)).await?;
    let key = super::ask("getBoardKey", true, || client.get_board_key())
        .await?
        .key;

    let mut problems = Vec::new();
    let index = |n: i64| usize::try_from(n).unwrap_or(usize::MAX);
    let published: Vec<ProvisionalEntry> = board
        .entries
        .iter()
        .map(|e| ProvisionalEntry {
            token: e.token.clone(),
            msg_randomizer: e.msg_randomizer.clone(),
            signature: e.signature.clone(),
            choices: e.choices.iter().map(|c| index(*c)).collect(),
        })
        .collect();
    let mut ballots = Vec::new();
    for entry in &published {
        let Ok((token, randomizer, signature)) = decode_bytes(entry) else {
            problems.push("a ballot on the board cannot be read".to_string());
            continue;
        };
        let msg_randomizer = randomizer
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .map(ballot_spec::MessageRandomizer);
        ballots.push(ballot_spec::BoardEntry {
            token,
            msg_randomizer,
            signature: ballot_spec::Signature(signature),
            choices: entry.choices.clone(),
        });
    }
    let issuer = view
        .issuer_pubkey
        .as_deref()
        .and_then(|b64| unb64(b64).ok())
        .and_then(|der| ballot_spec::IssuerPublicKey::from_der(&der).ok())
        .ok_or("the poll has no readable issuer key")?;
    let rules = ballot_spec::BallotRules {
        options: view.options.len(),
        min: index(view.min),
        max: index(view.max),
        blank: view.blank,
    };
    let counted = ballot_spec::recount(&issuer, &rules, ballots);
    for (at, why) in &counted.dropped {
        problems.push(format!("ballot {at} does not count: {why:?}"));
    }
    let announced: Vec<u64> = view
        .counts
        .unwrap_or_default()
        .iter()
        .map(|n| u64::try_from(*n).unwrap_or(0))
        .collect();
    if counted.counts != announced {
        problems.push(format!(
            "counted {:?}, and {announced:?} was announced",
            counted.counts
        ));
    }
    match (&view.closeout, &view.closed_at) {
        (Some(close), Some(closed_at)) => {
            let claimed = CloseOut {
                poll: poll.to_string(),
                entries: u64::try_from(close.entries).unwrap_or(0),
                issued: u64::try_from(close.issued).unwrap_or(0),
                counts: close
                    .counts
                    .iter()
                    .map(|n| u64::try_from(*n).unwrap_or(0))
                    .collect(),
                board_digest: close.board_digest.clone(),
                closed_at: closed_at.clone(),
            };
            if close.key != key || !verify(&key, &claimed.payload(), &close.sig) {
                problems.push("the close-out is not signed by this site's custody key".into());
            }
            if board_digest(&published) != claimed.board_digest {
                problems.push("the board is not the board that was signed for".into());
            }
            if claimed.entries > claimed.issued {
                problems.push("more ballots on the board than tokens were issued".into());
            }
        }
        _ if !view.open => problems.push("the poll is closed and nothing signs for it".into()),
        _ => {}
    }
    Ok(crate::model::Recounted {
        ballots: published.len(),
        problems,
    })
}
