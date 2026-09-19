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

/// A group carol made, with a submitted page in it. Returns their ids.
async fn a_group_with_a_page(carol: &str) -> (String, String) {
    let home = super::query_root_node(Some(carol), CAROL)
        .await
        .expect("the home")
        .expect("is there");
    let group = super::create_context(
        Some(carol),
        &home.id.0,
        &home.id.0,
        "wiki/group",
        "HB",
        None,
    )
    .await
    .expect("a group");
    let mut page = a_node("wiki/document", "Referat", &group.id.0, &group.id.0);
    page.mutable = Some(false);
    page.data = Some(crate::model::Jsonb(serde_json::json!({
        "content": [{"children": [{"text": "Mødet blev åbnet kl. 19"}]}]
    })));
    let page = super::insert_node(Some(carol), page)
        .await
        .expect("a page")
        .expect("inserted");
    (group.id.0, page.id.0)
}

#[tokio::test(flavor = "current_thread")]
async fn a_thread_is_written_read_and_taken_back_as_the_comments_component_does_it() {
    let server = Server::start();
    let carol = server.session(CAROL);
    let (group, page) = a_group_with_a_page(&carol).await;

    assert!(super::insert_comment(
        Some(&carol),
        &page,
        Some(&group),
        "k",
        "Carol",
        "Punkt 3 mangler",
        None
    )
    .await
    .expect("a comment"));
    let thread = super::query_comments(Some(&carol), &page)
        .await
        .expect("the thread");
    assert_eq!(thread.len(), 1);
    let first = &thread[0];
    assert_eq!(first.mime_id.as_deref(), Some("vote/comment"));
    assert_eq!(
        first.data.as_ref().expect("data").0["text"],
        "Punkt 3 mangler"
    );
    assert_eq!(
        (first.is_owner, first.is_context_owner),
        (Some(true), Some(true))
    );

    assert!(super::insert_comment(
        Some(&carol),
        &first.id.0,
        Some(&group),
        "k2",
        "Carol",
        "Rettet",
        None
    )
    .await
    .expect("an answer"));
    assert!(
        super::insert_reaction(Some(&carol), &first.id.0, Some(&group), "👍")
            .await
            .expect("a reaction")
    );
    let reactions = super::query_reactions(Some(&carol), &first.id.0)
        .await
        .expect("reactions");
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].data.as_ref().expect("data").0["emoji"], "👍");
    assert_eq!(
        reactions[0].owner_id,
        Some(Uuid(CAROL.into())),
        "whose it is, to mark it as hers"
    );

    // The feed draws a row from the node it is handed: what happened, the
    // comment it was to, and where.
    let feed = super::query_recent_nodes(Some(&carol), 20, 0, CAROL, Some(&group)).await;
    let kinds: Vec<&str> = feed
        .iter()
        .filter_map(|row| row.mime_id.as_deref())
        .collect();
    assert_eq!(
        kinds,
        [
            "vote/reaction",
            "vote/comment",
            "vote/comment",
            "wiki/document"
        ]
    );
    let reaction = &feed[0];
    let quoted = reaction.parent.as_ref().expect("what it is to");
    assert_eq!(quoted.mime_id.as_deref(), Some("vote/comment"));
    assert_eq!(
        quoted.data.as_ref().expect("data").0["text"],
        "Punkt 3 mangler"
    );
    assert_eq!(quoted.parent.as_ref().expect("where").name, "Referat");
    let opening =
        crate::components::content::slate_plain_text(&feed[3].data.as_ref().expect("data").0);
    assert_eq!(
        opening.trim(),
        "Mødet blev åbnet kl. 19",
        "how the page begins"
    );
    assert_eq!(
        super::thread_host(Some(&carol), &first.id.0).await,
        (page.clone(), Some("Referat".to_string()))
    );

    // Un-reacting is `delete_node` on the reaction's row, to the component.
    assert!(super::delete_node(Some(&carol), &reactions[0].id.0)
        .await
        .expect("un-react"));
    assert!(super::query_reactions(Some(&carol), &first.id.0)
        .await
        .expect("reactions")
        .is_empty());

    // An answered comment is emptied by an `update_node`, an unanswered one is
    // binned by a `bin_node`. Both are one method here, which decides itself.
    let answer = super::query_comments(Some(&carol), &first.id.0)
        .await
        .expect("answers")[0]
        .id
        .0
        .clone();
    let emptied = NodesSetInput {
        name: Some(String::new()),
        data: Some(crate::model::Jsonb(serde_json::json!({"deleted": true}))),
        ..Default::default()
    };
    assert!(super::update_node(Some(&carol), &first.id.0, emptied)
        .await
        .expect("emptied"));
    let thread = super::query_comments(Some(&carol), &page)
        .await
        .expect("the thread");
    assert_eq!(thread[0].data.as_ref().expect("data").0["deleted"], true);
    assert_eq!(
        super::bin_node(Some(&carol), &answer, None, None).await,
        Ok(1)
    );
    let bin = super::query_deleted(Some(&carol), &group, &group)
        .await
        .expect("the bin");
    assert_eq!(bin[0].mime_id.as_deref(), Some("vote/comment"));
    assert_eq!(super::restore_node(Some(&carol), &answer).await, Ok(1));

    let found = super::search_nodes(Some(&carol), "åbnet", None)
        .await
        .expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].parent.as_ref().expect("in").name, "HB");
    let hers = super::query_user_contributions(Some(&carol), CAROL, 10).await;
    assert!(
        hers.iter()
            .any(|row| row.mime_id.as_deref() == Some("vote/comment")),
        "{hers:?}"
    );
    assert!(super::query_orphans(Some(&carol))
        .await
        .expect("orphans")
        .is_empty());
}
