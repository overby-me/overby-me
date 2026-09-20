//! An AppView to develop against: an empty in-memory wiki with a home, served on
//! a local port, with a session already made for every DID named on the command
//! line. Signing in takes a PDS and a browser; this takes neither, which is what
//! lets the frontend's data layer be tested from `cargo test`, and the frontend
//! be run on a laptop.
//!
//!   appview-dev [--port N] [--db FILE] [--everyone-returns] <did>[=<address>] ...
//!
//! The first DID runs the site (owns the home). One line of JSON on stdout says
//! where it listens and which session is whose: `{"url": .., "sessions": {..}}`.
//! Never deployed: it mints sessions for anyone it is asked to.
//!
//! For rehearsing a cutover, `--db FILE` serves a datastore that `appview
//! import` filled (nobody is made its owner), `<did>=<address>` signs that DID
//! in as though its PDS had confirmed the address, which is how a person takes
//! their old account over, and `--everyone-returns` does that for every carried
//! account at once and says how it went, naming no address.

use appview::profile::PdsAccount;
use appview::{AppState, Config, Db, router};

/// The PDS every dev account is said to be on, and the one whose word on an
/// address is believed.
const DEV_PDS: &str = "https://pds.dev.invalid";

struct Asked {
    port: u16,
    db: Option<String>,
    everyone_returns: bool,
    /// `(did, the address its PDS confirms)`.
    people: Vec<(String, Option<String>)>,
}

fn asked() -> Option<Asked> {
    let mut asked = Asked {
        port: 0,
        db: None,
        everyone_returns: false,
        people: Vec::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => asked.port = args.next()?.parse().ok()?,
            "--db" => asked.db = Some(args.next()?),
            "--everyone-returns" => asked.everyone_returns = true,
            person if person.starts_with("did:") => {
                let (did, address) = match person.split_once('=') {
                    Some((did, address)) => (did, Some(address.to_string())),
                    None => (person, None),
                };
                asked.people.push((did.to_string(), address));
            }
            _ => return None,
        }
    }
    (!asked.people.is_empty() || asked.everyone_returns).then_some(asked)
}

/// Sign `did` in as its PDS would have: the account is made, and what a
/// confirmed address is owed (an old account, the seats waiting) is handed over.
async fn sign_in(state: &AppState, did: &str, address: Option<&str>) -> Result<u64, String> {
    appview::Store::new(state.db.clone())
        .upsert_user_min(did)
        .await
        .map_err(|e| e.to_string())?;
    let account = PdsAccount {
        pds: DEV_PDS.to_string(),
        confirmed_email: address.map(str::to_string),
        ..PdsAccount::default()
    };
    appview::profile::apply(state, did, &account)
        .await
        .map_err(|e| e.to_string())
}

/// Every carried account comes back under a DID of its own. One that fails is
/// a person who could not have signed in on the day.
async fn everyone_returns(state: &AppState) -> serde_json::Value {
    let mut carried: Vec<(String, String)> = Vec::new();
    {
        let conn = state.db.acquire().await.expect("a connection");
        let mut rows = conn
            .query("SELECT id, email FROM legacy_account ORDER BY id", ())
            .await
            .expect("the carried accounts");
        while let Some(row) = rows.next().await.expect("a row") {
            carried.push((row.get(0).expect("id"), row.get(1).expect("email")));
        }
    }
    let (mut seats, mut failed) = (0, Vec::new());
    for (i, (account, address)) in carried.iter().enumerate() {
        match sign_in(state, &format!("did:plc:returned{i}"), Some(address)).await {
            Ok(found) => seats += found,
            Err(e) => failed.push(serde_json::json!({ "account": account, "error": e })),
        }
    }
    let conn = state.db.acquire().await.expect("a connection");
    let mut rows = conn
        .query(
            "SELECT count(*) FROM member WHERE user_did IN (SELECT id FROM legacy_account)",
            (),
        )
        .await
        .expect("the seats still held");
    let still_held: i64 = match rows.next().await.expect("a row") {
        Some(row) => row.get(0).expect("a count"),
        None => 0,
    };
    serde_json::json!({
        "returned": carried.len() - failed.len(),
        "seats": seats,
        "seats_still_held_by_a_carried_account": still_held,
        "failed": failed,
    })
}

#[tokio::main]
async fn main() {
    let Some(asked) = asked() else {
        eprintln!(
            "usage: appview-dev [--port N] [--db FILE] [--everyone-returns] <did>[=<address>] ..."
        );
        std::process::exit(2);
    };

    // Bound first: the links the AppView hands out name where it listens.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", asked.port))
        .await
        .expect("a port");
    let url = format!("http://{}", listener.local_addr().expect("an address"));

    let db = Db::open(asked.db.as_deref().unwrap_or(":memory:"))
        .await
        .expect("a datastore");
    db.init_schema().await.expect("the schema");
    let from_env = Config::from_env();
    let config = Config {
        public_url: url.clone(),
        site_name: "Dev Wiki".to_string(),
        // A loaded wiki has the owners it came with.
        site_owner: match asked.db {
            Some(_) => None,
            None => asked.people.first().map(|(did, _)| did.clone()),
        },
        trusted_email_pds: vec!["pds.dev.invalid".to_string()],
        // So that a frontend served from elsewhere on this machine may call it
        // (`APPVIEW_FRONTEND_ORIGINS`, as the real one reads it).
        frontend_origins: from_env.frontend_origins,
        blob_dir: from_env.blob_dir,
        ..Config::default()
    };
    appview::context::ensure_home(&db, &config)
        .await
        .expect("a home");
    let state = AppState::new(db.clone(), config);
    // A load indexes nothing: the service does, each time it starts.
    if asked.db.is_some() {
        let indexed = appview::search::rebuild(&db).await.expect("a search index");
        eprintln!("search index rebuilt over {indexed} nodes");
    }

    let returned = match asked.everyone_returns {
        true => Some(everyone_returns(&state).await),
        false => None,
    };

    let mut sessions = serde_json::Map::new();
    for (did, address) in &asked.people {
        let seats = sign_in(&state, did, address.as_deref())
            .await
            .expect("a signed-in account");
        if address.is_some() {
            eprintln!("{did} signed in and found {seats} seats waiting");
        }
        // A real account's name comes from its PDS, which a dev one has not got:
        // `did:plc:carol` is Carol, `carol.test`, so that a page shows who wrote it.
        let name = did.rsplit(':').next().unwrap_or(did);
        let shown: String = name
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
            .collect();
        db.acquire()
            .await
            .expect("a connection")
            .execute(
                "UPDATE user SET display_name = coalesce(display_name, ?2), \
                   handle = coalesce(handle, ?3) WHERE did = ?1",
                [did.as_str(), shown.as_str(), &format!("{name}.test")],
            )
            .await
            .expect("a name");
        let session = appview::session::Sessions::new(db.clone())
            .create(did)
            .await
            .expect("a session");
        sessions.insert(did.clone(), serde_json::Value::String(session));
    }

    let mut hello = serde_json::json!({ "url": url, "sessions": sessions });
    if let Some(returned) = returned {
        hello["everyone_returns"] = returned;
    }
    println!("{hello}");
    axum::serve(listener, router(state)).await.expect("serving");
}
