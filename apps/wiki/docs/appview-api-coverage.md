# What the frontend asks for, and what answers it

Every data call the frontend makes today, against the AppView method that
replaces it. This is what the client behind `src/model.rs` is written from
(roadmap M9), and it is how four things the plan had missed came to light: no
way to start a group, reactions nobody gated, the canvas, and the pickers.

The frontend's data layer is `src/graphql/*.rs` (Hasura), `src/backend_api.rs`
(the sidecar) and `src/nhost.rs` (auth and storage). Live queries are
`src/graphql/subscriptions.rs`.

## Reading the tree

| Frontend | AppView |
|-|-|
| `resolve_path`, `node_path`, `path_from_id`, `path_crumbs` | `getNode` (a node by path or id, with crumbs), `resolveNode` |
| `query_node_by_id`, `query_children`, `query_drawer_children` | `getNode`, `listChildren` |
| `query_root_node` | `listContexts` (the site is the root) |
| `query_contexts` (my groups, my events) | `listContexts?scope=mine&kind=` |
| `query_public_places` | `listContexts?scope=public` |
| `node_insert_mimes`, `query_permissions` | `getNode`'s `viewer.can_create` |
| `is_descendant_of` | checked by `moveDocument` and `copyDocument` themselves |
| `search_nodes` | `search` |
| `query_recent_nodes`, `thread_host`, `thread_host_id` | `listRecent` (each row carries what it is about) |
| `query_user_contributions`, `query_group_contributions` | `listContributions?did=` and `?context=` |
| `query_orphans` | `listOrphans` |
| `query_nodes_by_ids` | none: the feed refetches a page |
| `count_nodes` | unused in the frontend |

## Writing the tree

| Frontend | AppView |
|-|-|
| `insert_node`, `insert_node_named` | `createDocument` |
| `create_context` (four writes) | `createContext` |
| `update_node` | `updateDocument`, `updateContext` |
| `set_context_attachable`, `set_context_public` | `updateContext` |
| `bin_node`, `delete_node`, `delete_node_deep` | `deleteDocument`, `deleteContext` |
| `restore_node`, `purge_node`, `query_deleted` | `restoreDocument`, `restoreContext`, `purgeDocument`, `listDeleted` |
| `deep_copy_node` (a request per node) | `copyDocument` |
| (moving is an `update_node` of `parentId`) | `moveDocument` |
| `set_node_authors`, `delete_node_members` | `setDocumentAuthors` |

## People

| Frontend | AppView |
|-|-|
| `query_members_page`, `count_active_members`, `is_active_member` | `listMembers`, `getVoterCount`, `getNode`'s `viewer.can_vote` |
| `invite_member`, `invite_members`, `invite_member_by_node` | `inviteMembers` (an invitation names an address, a name or a DID) |
| `update_member`, `remove_member` | `updateMember`, `removeMember` |
| `query_invitations`, `accept_invitation`, `accept_existing_member`, `decline_invitation` | `listInvitations`, `acceptInvitation`, `removeMember` on one's own row |
| `claim_membership`, `member_claim_link` (sidecar) | `claimMembership`, `getMemberClaimLink` |
| `parse_roster` (sidecar) | `parseRoster` |
| `query_user`, `query_users_by_ids` | `getProfile`; the `profiles` map most answers carry |
| `search_users`, `search_authors` | `searchPeople`, with `contexts=true` for the author picker |
| `search_bsky_actors` | none needed: the browser asks Bluesky's public API itself |

## Meetings

| Frontend | AppView |
|-|-|
| `create_speaker_list` and the `speak/*` nodes | the `*SpeakerList` and `*Speaker` methods |
| `active_node_id`, `set_active_relation`, `screen_*`, `set_screen_*` | `getProjector`, `setProjector` |
| `pixel.rs` (`create_canvas`, `load_canvas`, `paint_cell`, `my_last_paint`, `set_canvas_open`) | `createCanvas`, `getCanvas`, `paintCell`, `setCanvasOpen` |
| `focused_canvas`, `set_focused_canvas` | `setProjector` (the node on screen can be a canvas) |
| `create_poll`, `query_context_polls` | `openPoll`, `listPolls` |
| `cast_vote` | `castOpenBallot` |
| `vote_cast_secret`, `vote_status` (sidecar) | `issueBallotTokens` then `castBallot`; `getPoll`'s `viewer.issued` |
| `poll_tally`, `poll_vote_count`, `query_poll_votes` | `getPoll` (`counts`, `ballots`, `eligible`), `getBoard`, `getBoardEntry` |
| (closing a poll is an `update_node` of `mutable`) | `closePoll` |

## Talk

| Frontend | AppView |
|-|-|
| `insert_comment` (with its `data.image`), `query_comments` | `postComment` (`image`), `getComments` |
| `tombstone_comment`, `delete_comment_subtree` (`bin_node` on a comment) | `deleteComment`, which empties an answered comment and bins any other |
| a comment in the bin (`query_deleted`, `restore_node`, `purge_node`) | `listDeleted`, `restoreComment`, `purgeComment` |
| `insert_reaction`, `query_reactions` | `addReaction`, `removeReaction`, `getReactions` |
| `insert_feedback`, `query_feedback`, `report_error` (sidecar) | `submitFeedback`, `listFeedback`, `deleteFeedback` |
| `push_subscribe`, `push_unsubscribe`, `push_notify`, `push_reply` (sidecar) | `subscribePush`, `unsubscribePush`, `notifyContext`, `notifyReply` |
| `atproto_post` (sidecar) | `shareToBluesky` |

## Files

| Frontend | AppView |
|-|-|
| `nhost.rs` upload | `uploadBlob` |
| `file_url`, `file_bytes` | `GET /blob/<id>` with the session |
| `presigned_file_url`, `office_embed_url` | `getBlobLink` |
| `render_metafile` (sidecar) | `renderMetafile` |

## Signing in

| Frontend | AppView |
|-|-|
| NHost sign-in, sign-up, refresh | `GET /login`, `createSession`, `getSession`, `deleteSession` |
| `atproto_start_url`, `atproto_status`, `atproto_unlink` (linking a Bluesky account TO an NHost one) | gone: the DID is the account |

## Live

| Frontend | AppView |
|-|-|
| `subscriptions.rs`: a Hasura live query per view, each carrying rows | `/ws`: `context:<id>`, `user:<did>`, `public`. A change names what changed and never what it changed to, and the view refetches through the gated reads (`docs/use-live-topic-inventory.md`) |
| `cell_stream` (painted cells, streamed) | a `canvas` change, then `getCanvas?since=` |
| `state_stream` (a poll opening or closing, streamed) | a `poll` change, then `getPoll` |
| the frontend log shipper | `POST /log` |
