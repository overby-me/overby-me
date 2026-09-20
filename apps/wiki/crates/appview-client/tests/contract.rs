//! The contract between the lexicons, the AppView and this client, held by
//! running all three: the real router on a local port, driven through the
//! generated client with `strict` on, so that a field no lexicon names fails to
//! decode and so does one a lexicon promises and the server does not send.
//!
//! Every method is called at least once (`every_method_is_called_here` sees to
//! it), with an answer that succeeds where that can be had without a PDS or a
//! push service, and a refusal that is decoded where it cannot.

use appview::{AppState, Config, Db, router};
use appview_client::*;
use serde_json::json;

const CAROL: &str = "did:plc:carol";
const ALICE: &str = "did:plc:alice";
const BOB: &str = "did:plc:bob";

/// A wiki with a home that carol runs, served on a local port.
struct Wiki {
    state: AppState,
    client: Client,
}

impl Wiki {
    async fn start() -> Wiki {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        let blobs =
            std::env::temp_dir().join(format!("contract-{}", appview::util::random_token(8)));
        let config = Config {
            site_name: "Radikal Ungdom".to_string(),
            site_owner: Some(CAROL.to_string()),
            blob_dir: blobs.to_string_lossy().into_owned(),
            // Any scalar will do: nothing is sent where nobody has subscribed.
            vapid_private: appview::config::Secret::new(appview::util::b64url(&[7u8; 32])),
            ..Config::default()
        };
        appview::context::ensure_home(&db, &config)
            .await
            .expect("home");
        let state = AppState::new(db, config);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let app = router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Wiki {
            state,
            client: Client::new(&base),
        }
    }

    /// As `/callback` leaves it: a user row and a session. The OAuth dance
    /// itself takes a PDS.
    async fn sign_in(&self, did: &str) -> Client {
        appview::Store::new(self.state.db.clone())
            .upsert_user_min(did)
            .await
            .expect("user");
        let session = appview::session::Sessions::new(self.state.db.clone())
            .create(did)
            .await
            .expect("session");
        self.client.with_session(&session)
    }

    /// Carol's group under the home, with alice and bob seated in it: alice by
    /// a claim link, bob by accepting an invitation to his account.
    async fn with_group(&self) -> (Client, Client, Client, String) {
        let carol = self.sign_in(CAROL).await;
        let alice = self.sign_in(ALICE).await;
        let bob = self.sign_in(BOB).await;
        let home = carol
            .get_node(&get_node::Params {
                path: Some(String::new()),
                ..Default::default()
            })
            .await
            .expect("home");
        let get_node::OutputNode::Context(home) = home.node else {
            panic!("the empty path is the home");
        };
        let group = carol
            .create_context(&create_context::Input {
                kind: "group".into(),
                name: "Hovedbestyrelsen".into(),
                parent_id: home.id,
            })
            .await
            .expect("group");
        let invited = carol
            .invite_members(&invite_members::Input {
                context_id: group.id.clone(),
                invites: vec![
                    invite_members::InputInvitesItem {
                        email: Some("alice@wiki.example".into()),
                        name: Some("Alice".into()),
                        ..Default::default()
                    },
                    invite_members::InputInvitesItem {
                        did: Some(BOB.into()),
                        ..Default::default()
                    },
                ],
            })
            .await
            .expect("invite");
        assert_eq!(invited.inserted, 2);
        let roster = carol
            .list_members(&list_members::Params {
                context: group.id.clone(),
                ..Default::default()
            })
            .await
            .expect("members");
        let seat = |wanted: &str| {
            roster
                .members
                .iter()
                .find(|m| {
                    m.name.as_deref() == Some(wanted) || m.user_did.as_deref() == Some(wanted)
                })
                .unwrap_or_else(|| panic!("no seat for {wanted}"))
                .id
                .clone()
        };
        let link = carol
            .get_member_claim_link(&get_member_claim_link::Params {
                member: seat("Alice"),
            })
            .await
            .expect("link");
        alice
            .claim_membership(&claim_membership::Input { token: link.token })
            .await
            .expect("claim");
        let waiting = bob.list_invitations().await.expect("invitations");
        assert_eq!(waiting.invitations.len(), 1);
        bob.accept_invitation(&accept_invitation::Input {
            id: waiting.invitations[0].id.clone(),
        })
        .await
        .expect("accept");
        (carol, alice, bob, group.id)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tree() {
    let wiki = Wiki::start().await;
    let (carol, alice, _bob, group) = wiki.with_group().await;

    let folder = carol
        .create_document(&create_document::Input {
            context_id: group.clone(),
            kind: "folder".into(),
            title: "Bilag".into(),
            ..Default::default()
        })
        .await
        .expect("folder");
    let page = carol
        .create_document(&create_document::Input {
            context_id: group.clone(),
            parent_id: Some(folder.id.clone()),
            kind: "document".into(),
            title: "Dagsorden".into(),
            content: Some(json!([{"children": [{"text": "Valg af dirigent"}]}])),
            ..Default::default()
        })
        .await
        .expect("page");
    carol
        .update_document(&update_document::Input {
            id: page.id.clone(),
            title: Some("Dagsorden for mødet".into()),
            created_at: Some("2026-05-01".into()),
            ..Default::default()
        })
        .await
        .expect("update");
    carol
        .set_document_authors(&set_document_authors::Input {
            id: page.id.clone(),
            authors: vec![
                defs::AuthorView {
                    kind: "user".into(),
                    did: Some(ALICE.into()),
                    ..Default::default()
                },
                defs::AuthorView {
                    kind: "free_text".into(),
                    display: Some("Sekretariatet".into()),
                    ..Default::default()
                },
                defs::AuthorView {
                    kind: "context".into(),
                    context_id: Some(group.clone()),
                    ..Default::default()
                },
            ],
        })
        .await
        .expect("authors");

    let read = alice
        .get_node(&get_node::Params {
            path: Some(page.path.clone()),
            ..Default::default()
        })
        .await
        .expect("getNode");
    let get_node::OutputNode::Document(document) = &read.node else {
        panic!("a page is a document");
    };
    assert_eq!(document.title, "Dagsorden for mødet");
    assert_eq!(
        document.created_at.as_deref(),
        Some("2026-05-01T00:00:00.000Z")
    );
    assert_eq!(document.authors.len(), 3);
    assert_eq!(read.crumbs.len(), 3);
    assert!(read.viewer.is_member && !read.viewer.is_context_owner);

    let by_id = alice
        .resolve_node(&resolve_node::Params {
            path: page.path.clone(),
        })
        .await
        .expect("resolveNode");
    assert!(matches!(by_id, resolve_node::Output::Document(_)));
    alice
        .get_document(&get_document::Params {
            id: page.id.clone(),
        })
        .await
        .expect("getDocument");
    let listed = alice
        .list_children(&list_children::Params {
            parent: folder.id.clone(),
        })
        .await
        .expect("listChildren");
    assert_eq!((listed.children.len(), listed.documents.len()), (1, 1));
    let context = alice
        .get_context(&get_context::Params { id: group.clone() })
        .await
        .expect("getContext");
    assert_eq!(context.kind, "group");
    for scope in ["roots", "mine", "public"] {
        carol
            .list_contexts(&list_contexts::Params {
                scope: Some(scope.into()),
                ..Default::default()
            })
            .await
            .unwrap_or_else(|e| panic!("listContexts {scope}: {e}"));
    }

    carol
        .update_context(&update_context::Input {
            id: group.clone(),
            content: Some(json!([{"children": [{"text": "Om hovedbestyrelsen"}]}])),
            data: Some(json!({"image": "cover"})),
            visibility: Some("public".into()),
            ..Default::default()
        })
        .await
        .expect("updateContext");
    let open = wiki
        .client
        .get_node(&get_node::Params {
            id: Some(group.clone()),
            ..Default::default()
        })
        .await
        .expect("a public group, signed out");
    let get_node::OutputNode::Context(open) = open.node else {
        panic!("a group is a context");
    };
    assert_eq!(open.data, Some(json!({"image": "cover"})));

    let copy = carol
        .copy_document(&copy_document::Input {
            id: folder.id.clone(),
            parent_id: group.clone(),
        })
        .await
        .expect("copy");
    assert_eq!(copy.copied, 2);
    let moved = carol
        .move_document(&move_document::Input {
            id: page.id.clone(),
            parent_id: group.clone(),
        })
        .await
        .expect("move");
    assert!(moved.path.ends_with("/dagsorden"), "{}", moved.path);

    let binned = carol
        .delete_document(&delete_document::Input {
            id: copy.id.clone(),
        })
        .await
        .expect("delete");
    assert_eq!(binned.binned, 2);
    let bin = carol
        .list_deleted(&list_deleted::Params {
            context: group.clone(),
        })
        .await
        .expect("listDeleted");
    assert_eq!(bin.deleted.len(), 1);
    carol
        .restore_document(&restore_document::Input {
            id: copy.id.clone(),
        })
        .await
        .expect("restore");
    carol
        .delete_document(&delete_document::Input {
            id: copy.id.clone(),
        })
        .await
        .expect("delete again");
    let purged = carol
        .purge_document(&purge_document::Input { id: copy.id })
        .await
        .expect("purge");
    assert_eq!(purged.purged, 2);

    let event = carol
        .create_context(&create_context::Input {
            kind: "event".into(),
            name: "Landsmøde".into(),
            parent_id: group.clone(),
        })
        .await
        .expect("event");
    carol
        .delete_context(&delete_context::Input {
            id: event.id.clone(),
        })
        .await
        .expect("deleteContext");
    carol
        .restore_context(&restore_context::Input { id: event.id })
        .await
        .expect("restoreContext");

    let orphans = carol.list_orphans().await.expect("listOrphans");
    assert!(orphans.orphans.is_empty());
    let refused = carol
        .purge_orphan(&purge_orphan::Input { id: page.id })
        .await
        .expect_err("a page in its place is no orphan");
    assert_eq!(refused.name(), Some("NotFound"));
    let refused = alice
        .list_orphans()
        .await
        .expect_err("alice does not run the site");
    assert_eq!(refused.name(), Some("Forbidden"));
}

/// A page to talk about, in the group.
async fn a_page(owner: &Client, group: &str, title: &str) -> create_document::Output {
    let page = owner
        .create_document(&create_document::Input {
            context_id: group.to_string(),
            kind: "document".into(),
            title: title.into(),
            content: Some(json!([{"children": [{"text": "Mødet blev åbnet kl. 19"}]}])),
            ..Default::default()
        })
        .await
        .expect("page");
    owner
        .update_document(&update_document::Input {
            id: page.id.clone(),
            mutable: Some(false),
            ..Default::default()
        })
        .await
        .expect("submit");
    page
}

#[tokio::test(flavor = "multi_thread")]
async fn the_talk() {
    let wiki = Wiki::start().await;
    let (carol, alice, bob, group) = wiki.with_group().await;
    let page = a_page(&carol, &group, "Referat").await;

    let said = alice
        .post_comment(&post_comment::Input {
            on_id: page.id.clone(),
            text: "Punkt 3 mangler".into(),
            ..Default::default()
        })
        .await
        .expect("postComment");
    let answer = bob
        .post_comment(&post_comment::Input {
            on_id: said.id.clone(),
            text: "Enig".into(),
            ..Default::default()
        })
        .await
        .expect("an answer");
    bob.add_reaction(&add_reaction::Input {
        subject: said.id.clone(),
        emoji: "👍".into(),
    })
    .await
    .expect("addReaction");
    let reactions = alice
        .get_reactions(&get_reactions::Params {
            subject: said.id.clone(),
        })
        .await
        .expect("getReactions");
    assert_eq!(reactions.reactions.len(), 1);
    assert_eq!(reactions.reactions[0].reactor_did.as_deref(), Some(BOB));

    let thread = alice
        .get_comments(&get_comments::Params {
            on: page.id.clone(),
        })
        .await
        .expect("getComments");
    assert_eq!(thread.comments.len(), 1);
    assert_eq!(
        thread.comments[0].root_id.as_deref(),
        Some(page.id.as_str())
    );

    let feed = alice
        .list_recent(&list_recent::Params {
            context: Some(group.clone()),
            ..Default::default()
        })
        .await
        .expect("listRecent");
    let kinds: Vec<&str> = feed.items.iter().map(|i| i.node.as_str()).collect();
    assert_eq!(kinds, ["reaction", "comment", "comment", "document"]);
    assert_eq!(
        feed.items[0].quote.as_ref().expect("quote").text,
        "Punkt 3 mangler"
    );
    assert_eq!(
        feed.items[3].excerpt.as_deref(),
        Some("Mødet blev åbnet kl. 19")
    );
    let theirs = alice
        .list_contributions(&list_contributions::Params {
            did: Some(ALICE.into()),
            ..Default::default()
        })
        .await
        .expect("listContributions");
    assert_eq!(theirs.items.len(), 1);
    let hits = alice
        .search(&search::Params {
            q: "ÅBNET".into(),
            ..Default::default()
        })
        .await
        .expect("search");
    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].matched, "text");

    bob.remove_reaction(&remove_reaction::Input {
        subject: said.id.clone(),
        emoji: "👍".into(),
    })
    .await
    .expect("removeReaction");
    let emptied = alice
        .delete_comment(&delete_comment::Input {
            id: said.id.clone(),
        })
        .await
        .expect("an answered comment is emptied");
    assert_eq!(emptied.outcome, "emptied");
    let binned = bob
        .delete_comment(&delete_comment::Input {
            id: answer.id.clone(),
        })
        .await
        .expect("an unanswered one is binned");
    assert_eq!(binned.outcome, "binned");
    let bin = carol
        .list_deleted(&list_deleted::Params {
            context: group.clone(),
        })
        .await
        .expect("listDeleted");
    assert_eq!(bin.deleted[0].node, "comment");
    bob.restore_comment(&restore_comment::Input {
        id: answer.id.clone(),
    })
    .await
    .expect("restoreComment");
    bob.delete_comment(&delete_comment::Input {
        id: answer.id.clone(),
    })
    .await
    .expect("and binned again");
    let purged = bob
        .purge_comment(&purge_comment::Input { id: answer.id })
        .await
        .expect("purgeComment");
    assert_eq!(purged.purged, 1);

    // No push key is configured, so there is nobody to send to, which is an
    // answer and not a failure.
    let told = carol
        .notify_context(&notify_context::Input {
            context_id: group.clone(),
            title: Some("Referatet er ude".into()),
            ..Default::default()
        })
        .await
        .expect("notifyContext");
    assert_eq!(told.sent, 0);
    alice
        .notify_reply(&notify_reply::Input {
            parent: page.id,
            ..Default::default()
        })
        .await
        .expect("notifyReply");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_members() {
    let wiki = Wiki::start().await;
    let (carol, alice, bob, group) = wiki.with_group().await;

    let roster = carol
        .list_members(&list_members::Params {
            context: group.clone(),
            q: Some("ali".into()),
            ..Default::default()
        })
        .await
        .expect("listMembers");
    assert_eq!(roster.total, 1);
    let seat = roster.members[0].clone();
    assert_eq!(
        seat.email.as_deref(),
        Some("alice@wiki.example"),
        "an owner sees addresses"
    );
    let as_member = bob
        .list_members(&list_members::Params {
            context: group.clone(),
            ..Default::default()
        })
        .await
        .expect("a member reads the roster");
    assert!(
        as_member.members.iter().all(|m| m.email.is_none()),
        "and no addresses"
    );

    // This AppView sends no mail, and says so rather than seeming to.
    let unsent = carol
        .send_invitation(&send_invitation::Input {
            member: seat.id.clone(),
        })
        .await
        .expect_err("no mail is configured here");
    assert_eq!(unsent.name(), Some("MailNotConfigured"));

    carol
        .update_member(&update_member::Input {
            id: seat.id.clone(),
            active: Some(false),
            ..Default::default()
        })
        .await
        .expect("updateMember");
    let voters = carol
        .get_voter_count(&get_voter_count::Params {
            context: group.clone(),
        })
        .await
        .expect("getVoterCount");
    assert_eq!(
        voters.count, 2,
        "carol and bob, with alice's vote taken away"
    );
    carol
        .remove_member(&remove_member::Input { id: seat.id })
        .await
        .expect("removeMember");
    let gone = alice
        .get_context(&get_context::Params { id: group.clone() })
        .await
        .expect_err("out of the group, she cannot read it");
    assert_eq!(gone.name(), Some("NotFound"));

    let me = bob.get_session().await.expect("getSession");
    assert_eq!(me.did, BOB);
    let her = bob
        .get_profile(&get_profile::Params { did: CAROL.into() })
        .await
        .expect("getProfile");
    assert_eq!(her.did, CAROL);
    let found = carol
        .search_people(&search_people::Params {
            q: "hoved".into(),
            contexts: Some(true),
        })
        .await
        .expect("searchPeople");
    assert_eq!(found.contexts.len(), 1);

    // Signing in takes a PDS: what can be held here is that a code nobody was
    // given is refused, and that signing out ends the session.
    let refused = wiki
        .client
        .create_session(&create_session::Input {
            code: "nobody's".into(),
        })
        .await
        .expect_err("createSession");
    assert_eq!(refused.name(), Some("InvalidCode"));
    assert!(bob.delete_session().await.expect("deleteSession").ok);
    let ended = bob.get_session().await.expect_err("the session is over");
    assert!(matches!(ended, Error::Api { status: 401, .. }), "{ended}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_meeting() {
    let wiki = Wiki::start().await;
    let (carol, alice, bob, group) = wiki.with_group().await;
    let page = a_page(&carol, &group, "Dagsorden").await;

    let list = carol
        .create_speaker_list(&create_speaker_list::Input {
            context_id: group.clone(),
            name: "Talerliste".into(),
        })
        .await
        .expect("createSpeakerList");
    carol
        .update_speaker_list(&update_speaker_list::Input {
            id: list.id.clone(),
            turn_secs: Some(120),
            ..Default::default()
        })
        .await
        .expect("updateSpeakerList");
    let first = alice
        .join_speaker_list(&join_speaker_list::Input {
            list_id: list.id.clone(),
            ..Default::default()
        })
        .await
        .expect("joinSpeakerList");
    let second = bob
        .join_speaker_list(&join_speaker_list::Input {
            list_id: list.id.clone(),
            kind: Some(1),
        })
        .await
        .expect("bob joins");
    carol
        .move_speaker(&move_speaker::Input {
            entry_id: second.id.clone(),
            to: "front".into(),
        })
        .await
        .expect("moveSpeaker");
    let lists = alice
        .list_speaker_lists(&list_speaker_lists::Params {
            context: group.clone(),
        })
        .await
        .expect("listSpeakerLists");
    assert_eq!(lists.lists[0].queue[0].speaker_did, BOB);
    assert_eq!(lists.lists[0].turn_secs, 120);
    let served = carol
        .next_speaker(&next_speaker::Input {
            list_id: list.id.clone(),
        })
        .await
        .expect("nextSpeaker");
    assert!(served.served.is_some());
    alice
        .leave_speaker_list(&leave_speaker_list::Input { entry_id: first.id })
        .await
        .expect("leaveSpeakerList");
    carol
        .clear_speaker_list(&clear_speaker_list::Input {
            list_id: list.id.clone(),
        })
        .await
        .expect("clearSpeakerList");
    carol
        .delete_speaker_list(&delete_speaker_list::Input { list_id: list.id })
        .await
        .expect("deleteSpeakerList");

    let shown = carol
        .set_projector(&set_projector::Input {
            context_id: group.clone(),
            active_id: Some(Some(page.id.clone())),
            show_comments: Some(true),
            ..Default::default()
        })
        .await
        .expect("setProjector");
    assert_eq!(shown.active_id.as_deref(), Some(page.id.as_str()));
    let cleared = carol
        .set_projector(&set_projector::Input {
            context_id: group.clone(),
            active_id: Some(None),
            ..Default::default()
        })
        .await
        .expect("clearing the screen");
    assert_eq!(
        cleared.active_id, None,
        "null clears, where absent would have left it"
    );
    let screen = bob
        .get_projector(&get_projector::Params {
            context: group.clone(),
        })
        .await
        .expect("getProjector");
    assert!(screen.show_comments && screen.active_id.is_none());

    let canvas = carol
        .create_canvas(&create_canvas::Input {
            parent_id: group.clone(),
            name: "Tavlen".into(),
            width: Some(8),
            height: Some(8),
            cooldown: Some(0),
        })
        .await
        .expect("createCanvas");
    assert!(canvas.path.ends_with("/tavlen"), "{}", canvas.path);
    bob.paint_cell(&paint_cell::Input {
        canvas: canvas.id.clone(),
        x: 3,
        y: 4,
        colour: 7,
    })
    .await
    .expect("paintCell");
    let board = alice
        .get_canvas(&get_canvas::Params {
            id: canvas.id.clone(),
            ..Default::default()
        })
        .await
        .expect("getCanvas");
    let cell = board.cells[0].as_array().expect("a row");
    assert_eq!(cell[..4], [json!(3), json!(4), json!(7), json!(0)]);
    assert_eq!(board.painters, [BOB]);
    let closed = carol
        .set_canvas_open(&set_canvas_open::Input {
            id: canvas.id,
            open: false,
        })
        .await
        .expect("setCanvasOpen");
    assert!(!closed.open);
}

/// The voter's side of a secret ballot, as a browser will run it.
fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .expect("base64url")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_vote() {
    let wiki = Wiki::start().await;
    let (carol, alice, bob, group) = wiki.with_group().await;
    let folder = carol
        .create_document(&create_document::Input {
            context_id: group.clone(),
            kind: "folder".into(),
            title: "Forslag".into(),
            ..Default::default()
        })
        .await
        .expect("folder");
    let motion = alice
        .create_document(&create_document::Input {
            context_id: group.clone(),
            parent_id: Some(folder.id),
            kind: "policy".into(),
            title: "Forslag 1".into(),
            ..Default::default()
        })
        .await
        .expect("motion");
    let open = |secret: bool| open_poll::Input {
        parent_id: motion.id.clone(),
        title: "Forslag 1".into(),
        options: vec!["for".into(), "imod".into(), "blank".into()],
        blank: Some(true),
        secret: Some(secret),
        ..Default::default()
    };

    // A vote given away, seen by both its ends, and taken back before anything
    // opens: what follows is each voting for themselves.
    alice
        .set_delegation(&set_delegation::Input {
            context_id: group.clone(),
            to_did: Some(Some(BOB.into())),
        })
        .await
        .expect("setDelegation");
    let given = bob
        .list_delegations(&list_delegations::Params {
            context: group.clone(),
        })
        .await
        .expect("listDelegations");
    assert_eq!((given.delegations.len(), given.all), (1, false));
    assert_eq!(given.delegations[0].from_did, ALICE);
    alice
        .set_delegation(&set_delegation::Input {
            context_id: group.clone(),
            to_did: Some(None),
        })
        .await
        .expect("taken back");

    let show_of_hands = carol.open_poll(&open(false)).await.expect("openPoll");
    assert!(show_of_hands.open && !show_of_hands.secret);
    bob.cast_open_ballot(&cast_open_ballot::Input {
        poll: show_of_hands.id.clone(),
        choices: vec![0],
    })
    .await
    .expect("castOpenBallot");
    let tally = alice
        .get_poll(&get_poll::Params {
            id: show_of_hands.id.clone(),
        })
        .await
        .expect("getPoll");
    assert_eq!((tally.ballots, tally.eligible), (1, 3));
    let result = carol
        .close_poll(&close_poll::Input {
            id: show_of_hands.id,
        })
        .await
        .expect("closePoll");
    assert_eq!(result.counts, Some(vec![1, 0, 0]));

    let secret = carol.open_poll(&open(true)).await.expect("a secret poll");
    let key = ballot_spec::IssuerPublicKey::from_der(&unb64(
        secret.issuer_pubkey.as_deref().expect("its issuer key"),
    ))
    .expect("der");
    let request = ballot_spec::request_token(&key).expect("blind");
    let issued = bob
        .issue_ballot_tokens(&issue_ballot_tokens::Input {
            poll: secret.id.clone(),
            blinded: vec![b64(&request.blinding.blind_message.0)],
        })
        .await
        .expect("issueBallotTokens");
    let signature = ballot_spec::finalize_token(
        &key,
        &request,
        &ballot_spec::BlindSignature(unb64(&issued.signatures[0])),
    )
    .expect("unblind");
    let entry = ballot_spec::provisional::encode_entry(&ballot_spec::BoardEntry {
        token: request.nullifier.clone(),
        msg_randomizer: request.blinding.msg_randomizer,
        signature,
        choices: vec![1],
    });
    // No session: the token is the right to vote, and nothing ties it to bob.
    let cast = wiki
        .client
        .cast_ballot(&cast_ballot::Input {
            poll: secret.id.clone(),
            token: entry.token.clone(),
            msg_randomizer: entry.msg_randomizer.clone(),
            signature: entry.signature.clone(),
            choices: vec![1],
        })
        .await
        .expect("castBallot");
    assert_eq!(cast.position, 0);
    let board = alice
        .get_board(&get_board::Params {
            poll: secret.id.clone(),
        })
        .await
        .expect("getBoard");
    assert_eq!(board.entries.len(), 1);
    let mine = bob
        .get_board_entry(&get_board_entry::Params {
            poll: secret.id.clone(),
            token: entry.token,
        })
        .await
        .expect("getBoardEntry");
    assert_eq!(mine.entry.choices, [1]);
    let listed = alice
        .list_polls(&list_polls::Params {
            parent: Some(motion.id),
            ..Default::default()
        })
        .await
        .expect("listPolls");
    assert_eq!(listed.polls.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_files_and_the_rest() {
    let wiki = Wiki::start().await;
    let (carol, alice, bob, group) = wiki.with_group().await;
    // Both clocks are this machine's, so what is measured is the error.
    let ahead = appview_client::server_clock_ahead_ms().expect("every answer says the time");
    assert!(ahead.abs() < 1_000, "{ahead}");

    let picture = alice
        .upload_blob(
            &upload_blob::Params {
                context: group.clone(),
                name: Some("skærm.png".into()),
            },
            b"not really a png".to_vec(),
            "image/png",
        )
        .await
        .expect("uploadBlob");
    assert_eq!((picture.mime.as_str(), picture.size), ("image/png", 16));
    let link = bob
        .get_blob_link(&get_blob_link::Params {
            id: picture.id.clone(),
        })
        .await
        .expect("getBlobLink");
    assert!(link.url.contains(&picture.id), "{}", link.url);
    assert!(wiki.client.blob_url(&picture.id).ends_with(&picture.id));

    let report = alice
        .submit_feedback(&submit_feedback::Input {
            message: "Knappen er grå".into(),
            kind: Some("bug".into()),
            path: Some("/hb".into()),
            ..Default::default()
        })
        .await
        .expect("submitFeedback");
    let hers = alice.list_feedback().await.expect("listFeedback");
    assert!(!hers.all && hers.feedback.len() == 1);
    let all = carol
        .list_feedback()
        .await
        .expect("who runs the site sees them all");
    assert!(all.all);
    carol
        .delete_feedback(&delete_feedback::Input { id: report.id })
        .await
        .expect("deleteFeedback");
    alice
        .delete_blob(&delete_blob::Input { id: picture.id })
        .await
        .expect("deleteBlob");

    // What needs the outside world is held to its refusal: a push service, a
    // spreadsheet, a metafile and a PDS are not to be had here.
    let refused = bob
        .subscribe_push(&subscribe_push::Input {
            endpoint: "https://push.example/abc".into(),
            p256dh: "short".into(),
            auth: "short".into(),
        })
        .await
        .expect_err("subscribePush");
    assert_eq!(refused.name(), Some("InvalidRequest"));
    assert!(
        bob.unsubscribe_push(&unsubscribe_push::Input {
            endpoint: "https://push.example/abc".into(),
        })
        .await
        .expect("unsubscribePush")
        .ok
    );
    let sheet = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
    let refused = carol
        .parse_roster(b"no spreadsheet".to_vec(), sheet)
        .await
        .expect_err("parseRoster");
    assert_eq!(refused.name(), Some("InvalidRequest"));
    let refused = carol
        .render_metafile(b"no metafile".to_vec(), "image/x-emf")
        .await
        .expect_err("renderMetafile");
    assert!(matches!(refused, Error::Api { .. }), "{refused}");
    let refused = carol
        .share_to_bluesky(&share_to_bluesky::Input {
            text: "Referatet er ude".into(),
            ..Default::default()
        })
        .await
        .expect_err("shareToBluesky");
    assert!(matches!(refused, Error::Api { .. }), "{refused}");
}

/// The tests mean nothing if an unknown field slips through them.
#[test]
fn a_field_no_lexicon_names_is_refused_here() {
    let said = json!({"id": "d-1", "slug": "side", "path": "side", "unheard_of": true});
    assert!(serde_json::from_value::<create_document::Output>(said).is_err());
}

/// A new lexicon comes with a call here, or this fails: the contract is only as
/// wide as what is called.
#[test]
fn every_method_is_called_here() {
    let generated = include_str!("../src/generated.rs");
    let here = include_str!("contract.rs");
    let uncalled: Vec<&str> = generated
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub async fn "))
        .filter_map(|rest| rest.split('(').next())
        .filter(|method| !here.contains(&format!(".{method}(")))
        .collect();
    assert!(uncalled.is_empty(), "never called: {uncalled:?}");
}

/// The client in the tree is what the lexicons generate. Compared without
/// whitespace and commas, which are rustfmt's to move.
#[test]
fn the_client_is_what_the_lexicons_generate() {
    let lexicons =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lexicons/com/example/wiki");
    let fresh = lexgen::generate(&lexgen::read_lexicons(&lexicons).expect("lexicons"));
    let bare = |text: &str| -> String {
        text.chars()
            .filter(|c| !c.is_whitespace() && *c != ',')
            .collect()
    };
    assert!(
        bare(&fresh) == bare(include_str!("../src/generated.rs")),
        "generated.rs is stale: run `cargo run -p lexgen`"
    );
}
