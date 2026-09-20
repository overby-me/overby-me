//! Who is where: a context's roster, the caller's invitations, the pickers that
//! find a person or a group, the places a reader has, and who a place is open to.

use super::seen::saw_node;
use super::{ask, ask_quiet, client, map, reported};
use crate::model::{
    Author, ContextNodeFields, InvitationFields, MemberFields, MemberPageFilter, MembersSetInput,
    ParentNodeFields, PermissionFields, PublicPlace, UserSearchFields, Uuid,
};
use appview_client::{
    accept_invitation, defs, get_context, get_node, get_profile, get_voter_count, invite_members,
    list_contexts, list_members, remove_member, search_people, set_document_authors,
    update_context, update_member,
};

/// What a roster import did. `skipped` names people this context had already
/// invited: a roster says who belongs here, not that none of them are here yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RosterImport {
    pub inserted: usize,
    pub skipped: usize,
    /// Of those inserted, how many the file gave no address for.
    pub without_email: usize,
    /// Of those inserted, how many are being mailed a link to their seat.
    pub mailing: usize,
}

/// A page of a context's roster, and how many match in all.
pub async fn query_members_page(
    access_token: Option<&str>,
    parent_id: &str,
    filter: &MemberPageFilter,
    limit: usize,
    offset: usize,
) -> Result<(Vec<MemberFields>, usize), String> {
    let client = client(access_token);
    let params = list_members::Params {
        context: parent_id.to_string(),
        owner: filter.owner,
        active: filter.active,
        accepted: filter.accepted,
        hidden: filter.hidden,
        q: Some(filter.search.trim().to_string()).filter(|q| !q.is_empty()),
        limit: i64::try_from(limit).ok(),
        offset: i64::try_from(offset).ok(),
    };
    let page = ask("listMembers", true, || client.list_members(&params)).await?;
    let total = usize::try_from(page.total).unwrap_or(0);
    Ok((page.members.iter().map(map::member).collect(), total))
}

pub async fn update_member(
    access_token: Option<&str>,
    member_id: &str,
    set: MembersSetInput,
) -> Result<bool, String> {
    let client = client(access_token);
    // Saying yes to an invitation is an update of the row to the interim, and a
    // method of its own here: a seat is accepted by whoever it is for.
    if set.accepted == Some(true) {
        let yes = accept_invitation::Input {
            id: member_id.to_string(),
        };
        ask("acceptInvitation", false, || client.accept_invitation(&yes)).await?;
        return Ok(true);
    }
    let change = update_member::Input {
        id: member_id.to_string(),
        active: set.active,
        email: set.email,
        hidden: set.hidden,
        name: set.name,
        owner: set.owner,
    };
    ask("updateMember", false, || client.update_member(&change)).await?;
    Ok(true)
}

pub async fn remove_member(access_token: Option<&str>, member_id: &str) -> Result<bool, String> {
    let client = client(access_token);
    let gone = remove_member::Input {
        id: member_id.to_string(),
    };
    ask("removeMember", false, || client.remove_member(&gone)).await?;
    Ok(true)
}

async fn invite(
    access_token: Option<&str>,
    context_id: &str,
    invites: Vec<invite_members::InputInvitesItem>,
) -> Result<RosterImport, String> {
    let client = client(access_token);
    let roster = invite_members::Input {
        context_id: context_id.to_string(),
        invites,
    };
    let done = ask("inviteMembers", false, || client.invite_members(&roster)).await?;
    let count = |n: i64| usize::try_from(n).unwrap_or(0);
    Ok(RosterImport {
        inserted: count(done.inserted),
        skipped: count(done.skipped),
        without_email: count(done.without_email),
        mailing: count(done.mailing),
    })
}

pub async fn invite_member(
    access_token: Option<&str>,
    parent_id: &str,
    email: &str,
) -> Result<bool, String> {
    let one = invite_members::InputInvitesItem {
        email: Some(email.to_string()),
        ..Default::default()
    };
    Ok(invite(access_token, parent_id, vec![one]).await?.inserted > 0)
}

/// Invite an account. `node_id` is who: a DID here, where it was a user's id.
pub async fn invite_member_by_node(
    access_token: Option<&str>,
    parent_id: &str,
    node_id: &str,
    name: &str,
) -> Result<bool, String> {
    let one = invite_members::InputInvitesItem {
        did: Some(node_id.to_string()),
        name: Some(name.to_string()).filter(|n| !n.trim().is_empty()),
        ..Default::default()
    };
    Ok(invite(access_token, parent_id, vec![one]).await?.inserted > 0)
}

/// A whole roster, as `(name, address)` rows.
pub async fn invite_members(
    access_token: Option<&str>,
    parent_id: &str,
    roster: &[(String, String)],
) -> Result<RosterImport, String> {
    let rows = roster
        .iter()
        .map(|(name, email)| invite_members::InputInvitesItem {
            name: Some(name.clone()).filter(|n| !n.trim().is_empty()),
            email: Some(email.clone()).filter(|e| !e.trim().is_empty()),
            ..Default::default()
        })
        .collect();
    invite(access_token, parent_id, rows).await
}

/// The invitations waiting for the caller. The AppView knows who is asking and
/// what address their account has confirmed, so neither is sent.
pub async fn query_invitations(
    access_token: Option<&str>,
    _user_id: &str,
    _email: &str,
) -> Result<Vec<InvitationFields>, String> {
    let client = client(access_token);
    let waiting = ask("listInvitations", true, || client.list_invitations()).await?;
    Ok(waiting
        .invitations
        .iter()
        .map(|invitation| InvitationFields {
            id: Uuid(invitation.id.clone()),
            parent: Some(ParentNodeFields {
                id: Uuid(invitation.context_id.clone()),
                name: invitation.context_name.clone(),
                key: invitation
                    .context_path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                mime_id: Some(map::mime_of(&invitation.context_kind)),
                data: None,
                author_avatar: None,
                parent: None,
            }),
        })
        .collect())
}

pub async fn accept_invitation(
    access_token: Option<&str>,
    member_id: &str,
    _user_id: &str,
) -> Result<bool, String> {
    let client = client(access_token);
    let yes = accept_invitation::Input {
        id: member_id.to_string(),
    };
    ask("acceptInvitation", false, || client.accept_invitation(&yes)).await?;
    Ok(true)
}

/// The interim's way round a seat the caller already holds in that context,
/// which an invitation to their address would double. The AppView never makes
/// the second seat, so there is nothing to fall back from.
pub async fn accept_existing_member(
    _access_token: Option<&str>,
    _parent_id: &str,
    _node_id: &str,
    _invitation_id: &str,
) -> Result<bool, String> {
    Ok(true)
}

/// Declining is removing the seat, which whoever it is for may do.
pub async fn decline_invitation(
    access_token: Option<&str>,
    member_id: &str,
) -> Result<bool, String> {
    remove_member(access_token, member_id).await
}

/// Whether the caller holds voting rights in a context. Advisory, as it was:
/// the server decides when the ballot arrives.
pub async fn is_active_member(
    access_token: Option<&str>,
    context_id: &str,
    _user_id: &str,
) -> Option<bool> {
    let params = get_node::Params {
        id: Some(context_id.to_string()),
        ..Default::default()
    };
    super::get_node(access_token, params)
        .await
        .ok()
        .map(|read| read.viewer.can_vote)
}

pub async fn count_active_members(access_token: Option<&str>, context_id: &str) -> usize {
    let client = client(access_token);
    let params = get_voter_count::Params {
        context: context_id.to_string(),
    };
    match ask("getVoterCount", true, || client.get_voter_count(&params)).await {
        Ok(voters) => usize::try_from(voters.count).unwrap_or(0),
        Err(_) => 0,
    }
}

/// Replace a page's author chips: an account, a group, or a name with no
/// account behind it.
pub async fn set_node_authors(
    access_token: Option<&str>,
    node_id: &str,
    authors: &[Author],
) -> Result<bool, String> {
    let client = client(access_token);
    let chips = authors
        .iter()
        .map(|author| match (&author.user_id, &author.node_id) {
            (Some(did), _) => defs::AuthorView {
                kind: "user".to_string(),
                did: Some(did.clone()),
                ..Default::default()
            },
            (None, Some(group)) => defs::AuthorView {
                kind: "context".to_string(),
                context_id: Some(group.clone()),
                ..Default::default()
            },
            (None, None) => defs::AuthorView {
                kind: "free_text".to_string(),
                display: Some(author.name.clone()),
                ..Default::default()
            },
        })
        .collect();
    let set = set_document_authors::Input {
        id: node_id.to_string(),
        authors: chips,
    };
    ask("setDocumentAuthors", false, || {
        client.set_document_authors(&set)
    })
    .await?;
    Ok(true)
}

fn person(user: &defs::UserView) -> Author {
    Author {
        name: user
            .display_name
            .clone()
            .or_else(|| user.handle.clone())
            .unwrap_or_default(),
        node_id: Some(user.did.clone()),
        avatar_url: user.avatar_url.clone().unwrap_or_default(),
        user_id: Some(user.did.clone()),
    }
}

async fn found(access_token: Option<&str>, query: &str, contexts: bool) -> Vec<Author> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let client = client(access_token);
    let params = search_people::Params {
        q: query.to_string(),
        contexts: Some(contexts),
    };
    let Ok(found) = ask("searchPeople", true, || client.search_people(&params)).await else {
        return Vec::new();
    };
    let groups = found.contexts.iter().map(|group| Author {
        name: group.title.clone(),
        node_id: Some(group.id.clone()),
        avatar_url: String::new(),
        user_id: None,
    });
    found.people.iter().map(person).chain(groups).collect()
}

/// People, for the invite box.
pub async fn search_users(access_token: Option<&str>, query: &str) -> Vec<Author> {
    found(access_token, query, false).await
}

/// People and groups, for the author chips: a branch puts a motion forward too.
pub async fn search_authors(access_token: Option<&str>, query: &str) -> Vec<Author> {
    found(access_token, query, true).await
}

pub async fn query_user(access_token: Option<&str>, id: &str) -> Option<UserSearchFields> {
    let client = client(access_token);
    let params = get_profile::Params {
        did: id.to_string(),
    };
    let user = ask_quiet(true, || client.get_profile(&params)).await.ok()?;
    Some(UserSearchFields {
        id: Uuid(user.did.clone()),
        display_name: person(&user).name,
        avatar_url: user.avatar_url.unwrap_or_default(),
    })
}

pub async fn query_users_by_ids(access_token: Option<&str>, ids: &[String]) -> Vec<Author> {
    let client = client(access_token);
    let asked = ids.iter().map(|id| {
        let client = client.clone();
        let params = get_profile::Params { did: id.clone() };
        async move { client.get_profile(&params).await.ok() }
    });
    futures_util::future::join_all(asked)
        .await
        .iter()
        .flatten()
        .map(person)
        .collect()
}

/// The caller's groups, events or sites: where they hold a seat they said yes
/// to, newest first.
pub async fn query_contexts(
    access_token: Option<&str>,
    _user_id: &str,
    mime_id: &str,
) -> Result<Vec<ContextNodeFields>, String> {
    let client = client(access_token);
    let params = list_contexts::Params {
        scope: Some("mine".to_string()),
        kind: map::kind_of(mime_id).map(str::to_string),
    };
    let mut mine = ask("listContexts", true, || client.list_contexts(&params))
        .await?
        .contexts;
    mine.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(mine
        .iter()
        .map(|context| {
            saw_node(&context.id, "context", &context.kind);
            map::context_node(context)
        })
        .collect())
}

/// The places a signed-out visitor may read.
pub async fn query_public_places(access_token: Option<&str>) -> Result<Vec<PublicPlace>, String> {
    let client = client(access_token);
    let params = list_contexts::Params {
        scope: Some("public".to_string()),
        ..Default::default()
    };
    let open = ask("listContexts", true, || client.list_contexts(&params)).await?;
    Ok(open
        .contexts
        .iter()
        .filter(|place| !place.path.is_empty())
        .map(|place| PublicPlace {
            id: place.id.clone(),
            name: place.name.clone(),
            path: place.path.clone(),
            mime_id: map::mime_of(&place.kind),
        })
        .collect())
}

/// Who may do what in a context, for the overview.
///
/// The interim keeps this as rows per context, seeded from one template, and
/// the AppView as one rule for every context (`crates/appview/src/authz.rs`),
/// which is set out here as the rows it would have been. The one thing that
/// does differ between contexts is whether it is open to everyone, and that row
/// is the one the screen's switch reads.
pub async fn query_permissions(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Vec<PermissionFields>, String> {
    let client = client(access_token);
    let params = get_context::Params {
        id: context_id.to_string(),
    };
    let context = match ask_quiet(true, || client.get_context(&params)).await {
        Ok(context) => context,
        Err(e) => return Err(reported("getContext", &e)),
    };
    let row = |mime: &str, role: &str, insert: bool| PermissionFields {
        id: Uuid(format!("{context_id}:{role}:{mime}")),
        mime_id: Some(mime.to_string()),
        role: role.to_string(),
        insert,
        select: true,
        delete: insert,
        active: true,
    };
    const MAKES: &[(&str, &str)] = &[
        ("wiki/folder", "owner"),
        ("wiki/document", "owner"),
        ("wiki/file", "owner"),
        ("vote/position", "owner"),
        ("vote/poll", "owner"),
        ("canvas/canvas", "owner"),
        ("vote/policy", "member"),
        ("vote/change", "member"),
        ("vote/candidate", "member"),
        ("vote/question", "member"),
        ("vote/comment", "member"),
        ("vote/reaction", "member"),
        ("vote/vote", "member"),
    ];
    let mut rows: Vec<PermissionFields> = MAKES
        .iter()
        .map(|(mime, role)| row(mime, role, true))
        .collect();
    if context.visibility == "public" {
        rows.push(row(&map::mime_of(&context.kind), "public", false));
    }
    Ok(rows)
}

/// Open a context, and what is in it, to everyone, or close it again.
pub async fn set_context_public(
    access_token: Option<&str>,
    context_id: &str,
    _context_mime: &str,
    on: bool,
) -> Result<(), String> {
    let client = client(access_token);
    let change = update_context::Input {
        id: context_id.to_string(),
        visibility: Some(if on { "public" } else { "private" }.to_string()),
        ..Default::default()
    };
    ask("updateContext", false, || client.update_context(&change)).await?;
    Ok(())
}
