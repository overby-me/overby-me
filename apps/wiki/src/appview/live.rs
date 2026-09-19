//! The data layer against a real AppView, from `cargo test`.
//!
//! Each test starts `appview-dev` (`crates/appview-dev`: an in-memory AppView
//! with a home, and sessions for whoever it is told to seat), and asks it the
//! questions the components ask, through the functions they call. What comes
//! back is what a component would be handed.
//!
//! `cargo test --features appview appview::live`. The first run builds the dev
//! server, which takes a while; after that it is a second per test.

use super::tests::URL;
use crate::model::{NodesInsertInput, NodesSetInput, Uuid};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

const CAROL: &str = "did:plc:carol";
const ALICE: &str = "did:plc:alice";

/// A dev server, stopped when the test is done with it.
struct Server {
    process: Child,
    sessions: serde_json::Value,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl Server {
    fn start() -> Server {
        let crates = concat!(env!("CARGO_MANIFEST_DIR"), "/crates");
        let built = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "-p", "appview-dev"])
            .current_dir(crates)
            .status()
            .expect("cargo");
        assert!(built.success(), "appview-dev did not build");
        let mut process = Command::new(format!("{crates}/target/debug/appview-dev"))
            .args([CAROL, ALICE])
            .stdout(Stdio::piped())
            .spawn()
            .expect("appview-dev");
        let mut said = String::new();
        BufReader::new(process.stdout.take().expect("stdout"))
            .read_line(&mut said)
            .expect("its first line");
        let said: serde_json::Value = serde_json::from_str(&said).expect("json");
        URL.with(|url| *url.borrow_mut() = said["url"].as_str().map(str::to_string));
        Server {
            process,
            sessions: said["sessions"].clone(),
        }
    }

    fn session(&self, did: &str) -> String {
        self.sessions[did].as_str().expect("a session").to_string()
    }
}

fn a_node(mime: &str, name: &str, parent: &str, context: &str) -> NodesInsertInput {
    NodesInsertInput {
        name: Some(name.to_string()),
        mime_id: Some(mime.to_string()),
        parent_id: Some(Uuid(parent.to_string())),
        context_id: Some(Uuid(context.to_string())),
        ..Default::default()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_group_is_made_written_in_and_read_back_as_the_components_read_it() {
    let server = Server::start();
    let carol = server.session(CAROL);
    let home = super::query_root_node(Some(&carol), CAROL)
        .await
        .expect("the home")
        .expect("is there");
    assert_eq!(home.mime_id.as_deref(), Some("wiki/home"));
    assert_eq!(home.is_context_owner, Some(true), "carol runs the site");
    assert_eq!(home.members.len(), 1, "and is listed as who does");

    let group = super::create_context(
        Some(&carol),
        &home.id.0,
        &home.id.0,
        "wiki/group",
        "Hovedbestyrelsen",
        None,
    )
    .await
    .expect("createContext");
    assert_eq!(group.key, "hovedbestyrelsen");
    let folder = super::insert_node(
        Some(&carol),
        a_node("wiki/folder", "Bilag", &group.id.0, &group.id.0),
    )
    .await
    .expect("a folder")
    .expect("inserted");
    let mut page = a_node("wiki/document", "Dagsorden", &folder.id.0, &group.id.0);
    page.data = Some(crate::model::Jsonb(serde_json::json!({
        "content": [{"children": [{"text": "Valg af dirigent"}]}], "image": "cover-1"
    })));
    let page = super::insert_node_named(Some(&carol), page, "Dagsorden")
        .await
        .expect("a page")
        .expect("inserted");
    assert_eq!(
        page.key, "dagsorden",
        "the key the AppView chose comes back"
    );

    let path = ["hovedbestyrelsen", "bilag", "dagsorden"].map(str::to_string);
    let read = super::resolve_path(Some(&carol), &path, CAROL)
        .await
        .expect("resolve")
        .expect("the page");
    assert_eq!(read.mime_id.as_deref(), Some("wiki/document"));
    let data = read.data.expect("data").0;
    assert_eq!(
        data["content"][0]["children"][0]["text"],
        "Valg af dirigent"
    );
    assert_eq!(data["image"], "cover-1");
    assert_eq!(read.is_owner, Some(true));

    let renamed = NodesSetInput {
        name: Some("Dagsorden for mødet".to_string()),
        mutable: Some(false),
        ..Default::default()
    };
    assert!(super::update_node(Some(&carol), &page.id.0, renamed)
        .await
        .expect("update"));
    let listed = super::query_children(Some(&carol), &folder.id.0, CAROL)
        .await
        .expect("children");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Dagsorden for mødet");
    assert!(!listed[0].mutable, "submitted");
    assert_eq!(listed[0].key, "dagsorden", "a rename keeps the address");
    let drawer = super::query_drawer_children(Some(&carol), &group.id.0, CAROL)
        .await
        .expect("drawer");
    assert_eq!((drawer.len(), drawer[0].child_count), (1, 1));

    let crumbs = super::path_crumbs(Some(&carol), &path)
        .await
        .expect("crumbs");
    let names: Vec<&str> = crumbs.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Hovedbestyrelsen", "Bilag", "Dagsorden for mødet"]);
    assert_eq!(super::node_path(Some(&carol), &page.id.0).await, path);
    assert!(super::is_descendant_of(Some(&carol), &page.id.0, &group.id.0).await);
    let offered = super::node_insert_mimes(Some(&carol), &folder.id.0).await;
    assert!(offered.contains(&"vote/policy".to_string()), "{offered:?}");

    // A closed group is not there for someone who is not in it, and that is
    // an answer, not a fault.
    let alice = server.session(ALICE);
    let hidden = super::resolve_path(Some(&alice), &path, ALICE)
        .await
        .expect("an answer");
    assert_eq!(hidden, None);
    let slugs = super::path_crumbs(Some(&alice), &path)
        .await
        .expect("crumbs");
    assert_eq!(slugs[0].name, "hovedbestyrelsen", "a trail of slugs");

    // The bin, and both ways out of it.
    assert_eq!(
        super::bin_node(Some(&carol), &folder.id.0, None, None).await,
        Ok(2)
    );
    let bin = super::query_deleted(Some(&carol), &group.id.0, &group.id.0)
        .await
        .expect("the bin");
    assert_eq!(bin.len(), 1);
    assert_eq!(bin[0].mime_id.as_deref(), Some("wiki/folder"));
    assert_eq!(super::restore_node(Some(&carol), &folder.id.0).await, Ok(2));
    assert_eq!(
        super::bin_node(Some(&carol), &folder.id.0, None, None).await,
        Ok(2)
    );
    assert_eq!(super::purge_node(Some(&carol), &folder.id.0).await, Ok(2));
}
