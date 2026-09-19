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

#[tokio::test(flavor = "current_thread")]
async fn a_roster_is_kept_and_an_invitation_answered_as_the_member_screens_do_it() {
    use crate::model::{MemberPageFilter, MembersSetInput};
    let server = Server::start();
    let (carol, alice) = (server.session(CAROL), server.session(ALICE));
    let (group, page) = a_group_with_a_page(&carol).await;

    assert!(
        super::invite_member_by_node(Some(&carol), &group, ALICE, "Alice")
            .await
            .expect("invite")
    );
    let roster = [
        ("Bo".to_string(), "bo@wiki.example".to_string()),
        ("Bo".to_string(), "BO@wiki.example ".to_string()),
    ];
    let imported = super::invite_members(Some(&carol), &group, &roster)
        .await
        .expect("a roster");
    assert_eq!(
        (imported.inserted, imported.skipped),
        (1, 1),
        "the same address twice is one seat"
    );

    let waiting = super::query_invitations(Some(&alice), ALICE, "")
        .await
        .expect("invitations");
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].parent.as_ref().expect("to what").name, "HB");
    assert_eq!(
        super::is_active_member(Some(&alice), &group, ALICE).await,
        Some(true)
    );
    assert!(
        super::accept_invitation(Some(&alice), &waiting[0].id.0, ALICE)
            .await
            .expect("accept")
    );
    let hers = super::query_contexts(Some(&alice), ALICE, "wiki/group")
        .await
        .expect("her groups");
    assert_eq!(hers.len(), 1);

    let everyone = MemberPageFilter::default();
    let (seats, total) = super::query_members_page(Some(&carol), &group, &everyone, 10, 0)
        .await
        .expect("the roster");
    assert_eq!((seats.len(), total), (3, 3));
    let owners = MemberPageFilter {
        owner: Some(true),
        ..Default::default()
    };
    let (seats, _) = super::query_members_page(Some(&carol), &group, &owners, 10, 0)
        .await
        .expect("owners");
    assert_eq!(seats.len(), 1);
    assert_eq!(seats[0].node_id, Some(Uuid(CAROL.into())));
    let (hers_seat, _) = super::query_members_page(
        Some(&carol),
        &group,
        &MemberPageFilter {
            search: "alice".into(),
            ..Default::default()
        },
        10,
        0,
    )
    .await
    .expect("by name");
    let no_vote = MembersSetInput {
        active: Some(false),
        ..Default::default()
    };
    assert!(
        super::update_member(Some(&carol), &hers_seat[0].id.0, no_vote)
            .await
            .expect("update")
    );
    assert_eq!(
        super::count_active_members(Some(&carol), &group).await,
        2,
        "carol and bo"
    );
    assert_eq!(
        super::is_active_member(Some(&alice), &group, ALICE).await,
        Some(false)
    );

    // Author chips: an account, a group, and a name with no account behind it.
    let chips = [
        crate::model::Author {
            name: "Alice".into(),
            node_id: Some(ALICE.into()),
            avatar_url: String::new(),
            user_id: Some(ALICE.into()),
        },
        crate::model::Author {
            name: "HB".into(),
            node_id: Some(group.clone()),
            avatar_url: String::new(),
            user_id: None,
        },
        crate::model::Author {
            name: "Sekretariatet".into(),
            node_id: None,
            avatar_url: String::new(),
            user_id: None,
        },
    ];
    assert!(super::set_node_authors(Some(&carol), &page, &chips)
        .await
        .expect("authors"));
    let read = super::query_node_by_id(Some(&carol), &page, CAROL)
        .await
        .expect("read")
        .expect("the page");
    let labels: Vec<String> = read
        .members
        .iter()
        .map(crate::model::MemberFields::label)
        .collect();
    assert_eq!(labels.len(), 3);
    assert_eq!(&labels[1..], ["HB", "Sekretariatet"]);

    let picked = super::search_authors(Some(&carol), "hb").await;
    assert!(
        picked
            .iter()
            .any(|a| a.node_id.as_deref() == Some(group.as_str())),
        "{picked:?}"
    );
    assert!(super::query_user(Some(&carol), ALICE).await.is_some());
    assert_eq!(
        super::query_users_by_ids(Some(&carol), &[CAROL.into(), ALICE.into()])
            .await
            .len(),
        2
    );

    // Open to everyone, as the permissions screen's switch reads and sets it.
    let is_public = |rows: &[crate::model::PermissionFields]| {
        rows.iter()
            .any(|p| p.role == "public" && p.select && p.active)
    };
    assert!(!is_public(
        &super::query_permissions(Some(&carol), &group)
            .await
            .expect("rules")
    ));
    super::set_context_public(Some(&carol), &group, "wiki/group", true)
        .await
        .expect("open");
    assert!(is_public(
        &super::query_permissions(Some(&carol), &group)
            .await
            .expect("rules")
    ));
    let open = super::query_public_places(None).await.expect("signed out");
    assert_eq!(open.len(), 1);
    assert_eq!(
        (open[0].path.as_str(), open[0].mime_id.as_str()),
        ("hb", "wiki/group")
    );

    assert!(super::decline_invitation(Some(&alice), &hers_seat[0].id.0)
        .await
        .expect("leave"));
    assert!(super::query_contexts(Some(&alice), ALICE, "wiki/group")
        .await
        .expect("her groups")
        .is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn a_show_of_hands_is_opened_cast_in_and_closed_as_the_vote_screens_do_it() {
    use crate::model::BallotRules;
    let server = Server::start();
    let carol = server.session(CAROL);
    let (group, _page) = a_group_with_a_page(&carol).await;
    let folder = super::insert_node(
        Some(&carol),
        a_node("wiki/folder", "Forslag", &group, &group),
    )
    .await
    .expect("a folder")
    .expect("inserted");
    let mut motion = a_node("vote/policy", "Forslag 1", &folder.id.0, &group);
    motion.mutable = Some(false);
    let motion = super::insert_node(Some(&carol), motion)
        .await
        .expect("a motion")
        .expect("inserted");

    let options = ["for", "imod", "blank"].map(str::to_string);
    let poll = super::create_poll(
        Some(&carol),
        &motion.id.0,
        &group,
        "Forslag 1",
        "afstemning",
        &options,
        1,
        1,
        BallotRules::default(),
    )
    .await
    .expect("createPoll");
    assert_eq!(
        super::active_node_id(Some(&carol), &group).await,
        Ok(Some(poll.id.0.clone())),
        "an opened poll goes on the room's screen"
    );

    // A poll is a node to the components: what is asked in `data`, open while
    // `mutable`, both on its own page and among the motion's children.
    let on_its_page = super::query_node_by_id(Some(&carol), &poll.id.0, CAROL)
        .await
        .expect("read")
        .expect("the poll");
    assert_eq!(on_its_page.mime_id.as_deref(), Some("vote/poll"));
    assert!(on_its_page.mutable, "open");
    let data = on_its_page.data.expect("data").0;
    assert_eq!(data["options"], serde_json::json!(["for", "imod", "blank"]));
    assert_eq!(
        (&data["minVote"], &data["secret"]),
        (&serde_json::json!(1), &serde_json::json!(false))
    );
    let motion_page = super::query_node_by_id(Some(&carol), &motion.id.0, CAROL)
        .await
        .expect("read")
        .expect("the motion");
    assert_eq!(
        motion_page.children[0].data.as_ref().expect("data").0["nodeId"],
        motion.id.0.as_str()
    );

    assert!(
        super::cast_vote(Some(&carol), &poll.id.0, Some(&group), &[0], "x")
            .await
            .expect("cast")
    );
    assert_eq!(
        super::poll_tally(Some(&carol), &poll.id.0, 3, Some(CAROL)).await,
        Ok((vec![1, 0, 0], 1, 1))
    );
    assert_eq!(
        super::poll_vote_count(Some(&carol), &poll.id.0).await,
        Ok(1)
    );
    assert_eq!(
        super::query_poll_votes(Some(&carol), &poll.id.0).await,
        Ok(vec![vec![0]])
    );

    // Closing is an `update_node` of `mutable` to the component.
    let close = NodesSetInput {
        mutable: Some(false),
        ..Default::default()
    };
    assert!(super::update_node(Some(&carol), &poll.id.0, close)
        .await
        .expect("close"));
    let listed = super::query_context_polls(Some(&carol), &group)
        .await
        .expect("polls");
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].mutable, "closed");

    // Opening the next one closes nothing that is closed, and takes the screen.
    let next = super::create_poll(
        Some(&carol),
        &motion.id.0,
        &group,
        "Forslag 1",
        "igen",
        &options,
        1,
        1,
        BallotRules::default(),
    )
    .await
    .expect("a second poll");
    assert_eq!(
        super::active_node_id(Some(&carol), &group).await,
        Ok(Some(next.id.0))
    );

    // The rest of the screen: an anchor, the talk and the feed beside it, a board.
    super::set_screen_focus(Some(&carol), &group, Some("punkt-3"))
        .await
        .expect("focus");
    assert_eq!(
        super::screen_focus_anchor(Some(&carol), &group)
            .await
            .as_deref(),
        Some("punkt-3")
    );
    assert!(super::set_screen_comments(Some(&carol), &group, true)
        .await
        .expect("comments"));
    assert_eq!(
        super::screen_comments_on(Some(&carol), &group).await,
        Ok(true)
    );
    assert_eq!(super::screen_feed_on(Some(&carol), &group).await, Ok(false));
    super::set_focused_canvas(Some(&carol), &group, Some("board"))
        .await
        .expect("board");
    assert_eq!(
        super::focused_canvas(Some(&carol), &group).await.as_deref(),
        Some("board")
    );
    assert!(super::set_active_relation(Some(&carol), &group, None)
        .await
        .expect("clear"));
    assert_eq!(super::active_node_id(Some(&carol), &group).await, Ok(None));
}
