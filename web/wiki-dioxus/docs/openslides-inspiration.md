# OpenSlides: Inspiration & Ideas for RadikalWiki

## What OpenSlides is, and why learn from it

[OpenSlides](https://openslides.com) is a mature, open-source (MIT) assembly-management system used to run parliamentary meetings, party conferences, and delegate assemblies. It handles the full lifecycle of a meeting: motions with amendments and diffs, electronic and paper ballots, elections, a live list of speakers, a projector/beamer surface for the room, participant/permission management, and a follow-along "Autopilot" for delegates on their phones. Its backend is a service-oriented, event-sourced architecture (an append-only datastore plus a permission-filtering push service).

It is the right thing to learn from for RadikalWiki because the two tools solve the same problem, running a legitimate deliberative assembly (a *landsmøde*), and because OpenSlides has already made and documented the hard modeling decisions RadikalWiki is now approaching: motion workflows, entitled-voter snapshots, named-vs-anonymous ballots, and, crucially, an append-only record store with a server-side restrictor that maps almost one-to-one onto the wiki's planned atproto/lexicon backend.

This document integrates a verified idea catalog. It has been filtered against three checks: OpenSlides accuracy (a few claims corrected below), novelty against the current wiki source (already-shipped items demoted or dropped, duplicates merged), and completeness (genuinely missing OpenSlides features added). Where the catalog overstated what OpenSlides does or what the wiki lacks, the text here is the corrected version.

## How the two models line up

| OpenSlides | RadikalWiki (nodes / mime / permissions) |
|---|---|
| Organization | `wiki/home` (root node) / the whole deployment |
| Committee | `wiki/group` (self-owned context holding a permission template) |
| Meeting | `wiki/event` (self-owned context; a landsmøde/assembly) |
| Agenda (tree of agenda_items) | A context's ordered content children (folders/documents), surfaced by the program and admin apps |
| `agenda_item.type` (common/internal/hidden) | `data.visibility` on content nodes + the existing mime `hidden` flag (proposed AGE-01) |
| Topic (standalone agenda entry) | `wiki/document` or `wiki/folder` under a context |
| Motion (lead motion) | `vote/policy` |
| Amendment (`lead_motion_id` set) | `vote/change` (tree; amendments-of-amendments already supported) |
| `motion_change_recommendation` (line range) | `vote/change` anchored to a Slate block path via `data.targetPath` (proposed MOT-04) |
| Motion workflow / state | `data.status` on `vote/policy` + a workflow JSON on the context (proposed MOT-01) |
| Recommendation (proposed next state) | `data.recommendation` on `vote/policy` (proposed MOT-03) |
| `motion_category` (tree, prefix) | `wiki/folder` container (+ `data.prefix`) |
| `motion_block` | `wiki/folder` with `data.isBlock` and one governing poll (proposed VOT-08) |
| Supporters / min supporters | A `vote/support` child mime or member relation on `vote/policy` (proposed MOT-07) |
| Poll | `vote/poll` |
| Option | `data.options` entries on `vote/poll` (or per-candidate) |
| Vote | `vote/vote` (immutable, one per member via uniqueness constraint) |
| Poll type analog / named / pseudoanonymous | `data.type` on `vote/poll`; existing `secret` flag; `owner_id` present vs suppressed (VOT-05) |
| `pollmethod` Y/YN/YNA/N | `data.method` on `vote/poll` (proposed VOT-01) |
| Poll states created/started/finished/published | `data.state` on `vote/poll` (subsumes the mutable open/closed + hidden flags) (VOT-02) |
| `entitled_users_at_stop` snapshot | `data.entitledAtClose` frozen on `vote/poll` at close (VOT-03) |
| 100%-base | `data.base` on `vote/poll` (VOT-04) |
| Assignment (election) | `vote/position` |
| Candidate (option -> user) | `vote/candidate` (optionally linking a member `node_id`) |
| `list_of_speakers` | `speak/list` |
| speaker (waiting/current/finished) | `speak/speak` with `data.beginTime`/`endTime`/`pause` (AGE-03) |
| `speech_state` (pro/contra/POO/interposed) | `data.speechType` on `speak/speak` (AGE-04) |
| Structure level (delegation/faction) | `data.levels` tag on members + `data.structureLevels` on the context (PAR-05) |
| Projector | `screen/projector` app driven by the context's `active` relation |
| Projection queue (current/preview/history) | `active` + `preview:N` + `history:N` relations on the context (PRJ-01) |
| `projector_message` / chyron / countdown | Stable overlay relations/data on the context (PRJ-02, PRJ-03) |
| Autopilot | The follow app extended to compose active node + ballot + speaker list (PRJ-04) |
| Group (bundle of permission strings) | A named role in the permissions table + `members.role` (PAR-01) |
| Permission string `[domain].can_[action]` | A permissions-table row (context, mime, role, parent_mimes, insert/select/update/delete) |
| `admin_group` / `default_group` | `owner` role / `member` (and future `guest`) role |
| `is_present` + `entitled_group_ids` | `members.active` AND a proposed `members.present` flag (PAR-02) |
| `vote_weight` | `members.voteWeight` gated by context `data.enableVoteWeight` (PAR-03) |
| `vote_delegated_to_id` (proxy) | `members.delegatedTo` (PAR-04) |
| `chat_group` (read/write group lists) | Role-scoped channels over the ephemeral service (CHAT-01) |
| Datastore (append-only events) | atproto/lexicon records as the immutable node log (PLT-01) |
| Autoupdate service (restrictor + diffs) | A Rust push service filtering by the context permissions/roles, replacing Hasura subscriptions (PLT-01) |
| ModelRequest declarative subscription | A typed nested subscription request in the Rust backend (PLT-02) |
| Calculated/derived fields | Live tally / projected view / `get_index` / chyron recomputed on the stream (PLT-03) |
| ICC service (ephemeral) | A Rust ephemeral channel for presence/reactions/notifications (PLT-04) |
| Global History / restore | Intrinsic node version history from the append-only log + restore action (PLT-05) |
| Auth ticket (JWT + refresh cookie) | DID-bound JWT + httpOnly refresh cookie in the Rust backend (PLT-06) |
| Search service (restrictor-filtered) | Stream-fed Postgres/Rust FTS over extracted Slate text, restrictor-filtered (PLT-08) |
| Media service (id-based serving) | NHost storage via file ids / `wiki/file` nodes (already present) |
| Per-meeting custom translations | `data.translations` override on the context over the en/da bundle (PLT-10) |
| PDF/CSV/XLSX exports | Existing ODT+CSV export pipeline, extended with minutes/results/diffs (PLT-09) |
| `meeting.clone` / `meeting.archive` | Deep-copy of a `wiki/event` subtree + a `data.archived` read-only flag (CTX-02) |

## What we already do well

RadikalWiki has real parity with OpenSlides on a large surface. These are already shipped, so treat them as a baseline, not a to-do list:

- Amendments as first-class nodes that form a tree, including amendments-of-amendments (`vote/change` parents `vote/change`), matching OpenSlides' `lead_motion_id` nesting.
- Ordinal A/B/C labeling of amendments via the computed `get_index`, analogous to OpenSlides' amendment prefixes. (Note: this is a render-time position, not a durable citable number, see MOT-10.)
- A unified poll entity spanning motions, elections/positions, and standalone ballots (`vote/poll` on `vote/policy`, `vote/change`, or `vote/position`), like OpenSlides' single poll model.
- Ballot options with min/max choice cardinality (`minVote`/`maxVote`), covering single- and multi-choice.
- Secret/pseudoanonymous ballots (the `secret` flag: no `owner_id`, backend tracks has-voted), the same field-suppression approach OpenSlides uses.
- One-vote-per-member enforced by a key uniqueness constraint on `vote/vote` (recently hardened), matching OpenSlides' voted-set double-vote guard and non-idempotent submission.
- Hidden poll results (counts visible only to the context owner until reveal), analogous to OpenSlides' finished-vs-published gating.
- Immutable cast votes (`vote/vote` `mutable=false`), like OpenSlides freezing vote records.
- Election posts with candidates and questions (`vote/position`, `vote/candidate`, `vote/question`).
- Speaker lists with live turns and current/next highlighting (`speak/list` + `speak/speak`), plus multiple lists per context managed by owners.
- A projector/screen app that projects the active node room-facing, with a read-only speaker rail, a presenter `focus:%anchor` relation to scroll/zoom a section, and a comments-on-screen toggle.
- A personal "follow the room" device view (the follow app), the seed of an Autopilot.
- A chair's run-the-meeting console (admin app): agenda list, project/stop, live tallies with close.
- A context-scoped, role-keyed permission model (context, mime, role, parent_mimes, CRUD), structurally equivalent to OpenSlides' group-based per-object access control, with `owner` vs `member` roles today.
- A member roster bound by `node_id` with accepted/active/owner/hidden states, search/filter/server-side pagination, invite via `insertMembers`, remove, and CSV export.
- Real-time subscriptions on children, `active`, `focus`, screen-comments, poll close, and live tallies.
- Rich-text content (Slate: h1-h6, paragraphs, blockquote, images, links); threaded comments gated by the permissions table; drag-and-drop folder reordering (the sort app); a numbered agenda timeline (the program app); Postgres full-text search over extracted rich text.
- ODT export of documents and CSV export of members; i18n (en/da) with a contrast-audited Material Design UI; PWA with push notifications on poll open; id-based media serving; an emerging atproto/DID identity link (Bluesky linking in the profile app).

---

## Motions & Amendments

### MOT-01: Motion workflow as an explicit state graph

**What OpenSlides does:** Every motion belongs to exactly one workflow, a directed graph of named states (submitted -> permitted -> accepted/rejected/withdrawn) with a `first_state` and next/previous/reset transitions. States carry behavior flags: `allow_create_poll`, `allow_support`, `allow_submitter_edit`, `set_number`, `merge_amendment_into_final`.

**Why it fits:** The wiki's own inventory lists "no workflow/status tracking" as the top gap. A landsmøde still moves proposals draft -> submitted -> debated -> decided, and that lifecycle is invisible today. This is the single highest-leverage addition.

**Model mapping:** Add a `data.status` string on `vote/policy` and `vote/change`, plus a `data.workflow` reference on the context (which already holds the permission template) as a small JSON blob: an ordered list of allowed states with per-state flags. Only the `owner` role may update `data.status` (via the update permission). `allow_create_poll` maps to whether the poll-insert permission is honored while a policy sits in that state. Storing the workflow on the context keeps it per-assembly.

**Feasibility / Priority:** Medium / High. Ship one fixed 3-4 state workflow (draft/submitted/accepted/rejected) first; a full customizable-workflow editor is over-scoped. Per-state visibility restrictions add permission-evaluation complexity, defer.

### MOT-02: Seed a default workflow per context

**What OpenSlides does:** It seeds two workflows automatically, a simple one (submitted -> accepted/rejected/not decided) and a more complex multi-step one. (Do not assert an exact state count for the complex one; "a simple and a complex workflow" is the accurate claim.)

**Why it fits:** The wiki already seeds a default permission template on context creation. A default workflow can be seeded the same way, giving assemblies a sensible lifecycle with zero configuration.

**Model mapping:** Extend the existing context-creation seed (which already writes the default permission rows) to also write a default `data.workflow` JSON: states `[draft, submitted, accepted, rejected, withdrawn]` with CSS color hints the M3 theme can reuse. State labels should be i18n keys (en/da) so Danish terminology (*landsmøde*, *forslag*) can differ.

**Feasibility / Priority:** Small / Medium. Depends on MOT-01.

### MOT-03: Committee recommendation separate from the decision state

**What OpenSlides does:** A "recommendation" is a proposed next state (any state whose `recommendation_label` is non-empty). A recommending body sets it; the assembly adopts it via a one-click `follow_recommendation` action.

**Why it fits:** Radikal Ungdom has a board (*hovedbestyrelse*) that pre-vets proposals. Surfacing "the board recommends acceptance" before the floor votes mirrors real practice and speeds decisions.

**Model mapping:** Add `data.recommendation` (a proposed status value) on `vote/policy`, editable only by the `owner` role. Render it distinctly from `data.status`. A "follow recommendation" owner action copies `recommendation` into `status` in one step. No new mime.

**Feasibility / Priority:** Small / Medium. Depends on MOT-01 (low value without statuses).

### MOT-04: Typed line-range change recommendations anchored to Slate blocks

**What OpenSlides does:** Automatic line numbering is the substrate for amendments; a `motion_change_recommendation` targets a `line_from`/`line_to` range with type `replacement`/`insertion`/`deletion`/`other`, validated against overlaps and rendered as an inline diff. (Amendments proper are also driven by a paragraph/text-mode, not only line ranges, so this maps most cleanly onto block-level anchoring.)

**Why it fits:** `vote/change` amendments today are free-form child nodes with no anchoring to the policy text. Anchoring makes amendments precise and debuggable on screen, a core parliamentary need.

**Model mapping:** Content is Slate, not line-based, so anchor to Slate block paths instead of line numbers: store `data.targetPath` (a stable block id/anchor) and `data.changeType` (`replacement`/`insertion`/`deletion`) on `vote/change`. The editor computes block ids; the screen/projector diff-renders original vs changed. Reuse the existing `focus:%anchor` mechanism (already keyed by heading id) to point at the targeted block.

**Feasibility / Priority:** Large / Medium. Slate block paths shift as the base doc is edited, so you need stable block ids. Start with "targets block X" metadata before rendering full inline diffs.

### MOT-05: Auto-merge accepted amendments into a derived final version

**What OpenSlides does:** Each state has `merge_amendment_into_final` (`do_merge`/`do_not_merge`/`undefined`); accepted-type states default to merging, so accepted amendments are folded into a "Final version" rendering alongside original/changed/diff views (with a manual "modified final version" override for conflicts).

**Why it fits:** After a landsmøde adopts amendments, someone produces the consolidated final text by hand today. Auto-merge produces the resolution automatically.

**Model mapping:** When a `vote/change` reaches `status=accepted` and targets a block (MOT-04), compute a derived "final" Slate view of the parent `vote/policy` by applying accepted changes in index order. Render as a fourth view mode in `content.rs`. No stored mutation of the base, it is a computed view (aligns with append-only atproto records).

**Feasibility / Priority:** Large / Low. Depends on MOT-04 and MOT-01. Competing overlapping amendments need a manual tiebreak.

### MOT-06: Four synchronized text views (original / changed / diff / final)

**What OpenSlides does:** The motion detail view offers Original, Changed, Diff, and Final representations, reused in the UI and in PDF export.

**Why it fits:** Gives debaters and the chair a shared, unambiguous view of what a policy says now versus what an amendment proposes, a recurring source of floor confusion.

**Model mapping:** Add a view-mode toggle to `content.rs`/`policy.rs` rendering the `vote/policy` Slate content in four modes derived from its `vote/change` children. Diff view highlights inserted/deleted Slate blocks. Ties into the existing ODT export.

**Feasibility / Priority:** Medium / Low. Depends on MOT-04. Diffing rich Slate structures is harder than diffing plain lines.

### MOT-07: Supporters and a minimum-supporters threshold

**What OpenSlides does:** Motions collect supporters (`can_support`), allowed only when the state has `allow_support=true`; a min-supporters setting can gate whether a motion is admitted. Supporters are modeled as a distinct `motion_supporter` object.

**Why it fits:** Youth-org procedure often requires a proposal to be seconded or gather N backers before it reaches the floor. A natural, low-cost civic feature.

**Model mapping:** Reuse the `vote/comment` insert pattern: a lightweight `vote/support` child mime (member role, parent `vote/policy`, insert+delete only) recording backers, gated by the permissions table exactly like `vote/comment`. Count children and compare against context `data.minSupporters`. Alternatively model supporters as a member-list relation to avoid a new mime.

**Feasibility / Priority:** Medium / Medium. Enable per-assembly to avoid friction.

### MOT-08: Category prefixes and motion blocks (the delta over what we already sort)

**What OpenSlides does:** Motions organize into a category tree (with prefixes prepended to identifiers) and an ordered call list; `motion_block` groups motions for batch voting.

**Why it fits:** A landsmøde agenda groups proposals by topic (e.g. "Klima", "Uddannelse"). The wiki *already* has folders parenting policies, a sort app that persists index for call-list ordering, and a numbered program timeline. So the genuinely new work is narrow: prefix labeling and block-grouping, not ordering.

**Model mapping:** Categories map to existing `wiki/folder` containers. Add `data.prefix` on a folder to prepend to child ordinal labels. Motion blocks map to a folder with one governing poll (see VOT-08). The sort app's existing persisted index covers call-list ordering with no change.

**Feasibility / Priority:** Small / Medium. Only the prefix labeling is new machinery.

### MOT-09: Surface the amendment nesting chain (UI polish, already modeled)

**What OpenSlides does:** An amendment may itself be amended (`lead_motion_id` points at another amendment), forming a tree; amendments carry a recognizable prefix.

**Why it fits:** The core mechanism already exists: the seed lets `vote/change` parent `vote/change`, so amendments-of-amendments nest today, and `get_index` yields ordinals. This is not a new feature, it is surfacing what is there.

**Model mapping:** Add UI to render nesting depth and the ordinal chain clearly (e.g. "Amendment B.2"). No schema or permission change.

**Feasibility / Priority:** Small / Low. A UI task, not a feature.

### MOT-10: Durable sequential motion numbers stamped on state change

**What OpenSlides does:** The workflow `set_number` flag assigns a stable, citable motion number (e.g. `F1`, `K-14`) when a motion enters a state that sets it, numbered per category with the category prefix, and frozen thereafter even if the call list is resorted. This is distinct from a render-time ordinal.

**Why it fits:** The floor and the official minutes reference proposals by a stable number ("Forslag K-14"). The wiki's `get_index` is a render-time position that changes on resort, so it cannot serve as a citable identifier. This is a real gap the catalog otherwise missed.

**Model mapping:** Add a `set_number` flag to workflow states (MOT-01). On first transition into such a state, stamp an immutable `data.number` on the `vote/policy`, composed from the category `data.prefix` (MOT-08) and a per-category counter. Never recomputed on resort.

**Feasibility / Priority:** Medium / Medium. Depends on MOT-01 and MOT-08.

### MOT-11: Motion forwarding between contexts (chapter -> national)

**What OpenSlides does:** `motion.create_forwarded` forwards a motion (and its amendments) from one committee's meeting into another meeting where the forwarder may create motions, governed by `forward_to_committee_ids`; backtracking shows the origin meeting and status.

**Why it fits:** This models a local chapter (*lokalforening*) forwarding a proposal up to the national landsmøde, or carrying an unfinished motion from one assembly to the next. Without it, cross-assembly proposal flow is manual copy-paste with no provenance. Explicitly a gap.

**Model mapping:** Create a derived `vote/policy` node in the target context that references its origin via a `forwarded-from` relation (or `data.forwardedFrom` node id), reusing the existing node-copy and permission machinery. Gate who may forward via a context permission.

**Feasibility / Priority:** Medium / Medium.

### MOT-12: Statute/bylaw amendment motions

**What OpenSlides does:** It distinguishes ordinary motions from statute-amendment motions built on `motion_statute_paragraph`, targeting a numbered paragraph of the org's statutes and diffing against that paragraph rather than free text.

**Why it fits:** Radikal Ungdom amends its *vedtægter* (bylaws) at the landsmøde, and these amendments have special status, often a higher majority, and a diff-against-the-statute presentation distinct from policy proposals.

**Model mapping:** A statute-document node whose numbered blocks are the amendable paragraphs; a `vote/change` variant (`data.isStatute=true`) anchored to a paragraph block (reusing MOT-04 anchoring). Apply a higher majority via VOT-04's base configuration.

**Feasibility / Priority:** Medium / Low. Depends on MOT-04 and VOT-04.

### MOT-13: Cross-cutting tags as a many-to-many facet

**What OpenSlides does:** A first-class `tag` object applies to motions, agenda items, and files for fast grouping and live filtering, orthogonal to the category tree.

**Why it fits:** Folders (MOT-08) are single-parent: a node lives in exactly one folder. Tags are many-to-many and cut across the tree (tag a policy both "Klima" and "Hastesag"). During a fast-moving landsmøde, filter chips ("all urgent", "all board-recommended") are a navigation aid the folder tree cannot provide.

**Model mapping:** A `data.tags` string list on any node plus a per-context tag palette (on the context `data`). Filter chips in the program and vote apps intersect the viewer's selection with `data.tags`.

**Feasibility / Priority:** Small / Low.

---

## Voting & Elections

### VOT-01: Poll methods Y / YN / YNA / N

**What OpenSlides does:** `pollmethod` is exactly `Y`/`YN`/`YNA`/`N`; each option accumulates decimal yes/no/abstain tallies. Motions default YNA (For/Against/Abstain), elections default Y.

**Why it fits:** Today ballot options are arbitrary strings (For/Against/Blank). Adopting the YNA structure standardizes motion voting and enables consistent result math and 100%-base percentages.

**Model mapping:** Add `data.method` on `vote/poll` alongside the existing `options`/`minVote`/`maxVote`. For YNA, options become a fixed `{yes,no,abstain}` shape and `vote/vote` records `data.value` from that set. Keep free-form options for list ballots. A `data.*` convention change, no schema migration. Add `method` as optional, defaulting to legacy free-options behavior.

**Feasibility / Priority:** Medium / High.

### VOT-02: Poll lifecycle state machine (created -> started -> finished -> published)

**What OpenSlides does:** Polls move `created -> started -> finished -> published` via explicit start/stop/publish/reset actions. Results are hidden (managers-only) until published; reset deletes all votes.

**Why it fits:** The wiki today has only a binary mutable flag (`open` = `node.mutable`; close sets `mutable:false`). A four-state machine cleanly separates "not yet open", "voting", "closed but results withheld", and "results public", which the chair needs to control reveal timing.

**Model mapping:** Add `data.state` on `vote/poll`, owner-only transitions gated by the update permission. `created` hides the ballot; `started` allows `vote/vote` inserts (permission honored only in this state); `finished` blocks inserts and hides counts from non-owners; `published` reveals counts. Subsumes the existing hidden flag.

**Feasibility / Priority:** Medium / High. The backend must enforce that vote inserts are only accepted in `started`, not just the UI. Do not model reset-deletes-votes: with append-only atproto records, model a reset as a new poll instead.

### VOT-03: Freeze results and an entitled-voters snapshot at close

**What OpenSlides does:** At `poll.stop`, `votescast`/`votesvalid` and an `entitled_users_at_stop` JSON snapshot (`{user_id, voted, present}`) are frozen, so later member deletion cannot alter historical results.

**Why it fits:** This is genuinely missing. Turnout today is computed live at render (`count_active_members`), so historical results shift if members are later removed or deactivated. A frozen snapshot gives the assembly a trustworthy governance record.

**Model mapping:** On close (VOT-02 `finished`), write `data.frozenResults` (counts) and `data.entitledAtClose` (list of active/present member `node_id`s at that instant) onto the `vote/poll`, made immutable (`mutable=false`). Turnout = votes / `entitledAtClose` length. This snapshot must be written server-side in a transaction (the trusted service, not the client). Aligns with the trusted-service-for-ballots atproto decision.

**Feasibility / Priority:** Medium / High. Depends on VOT-02.

### VOT-04: Selectable 100%-base for results presentation

**What OpenSlides does:** Result percentages compute on a selectable base: `Y`, `N`, `YN`, `YNA`, `valid`, `cast`, `entitled`, or `disabled`. Percentage = value / base * 100. (`entitled` already means present-and-entitled; there is no separate `entitled_present` base.)

**Why it fits:** The wiki already renders a turnout percentage against active members, so this is not "introduce percentages", it is making the *base* selectable. Different motions need different denominators (of votes cast vs of all entitled members), and making the base explicit avoids disputes about whether a proposal passed.

**Model mapping:** Add `data.base` on `vote/poll` (default `cast`). The tally renderer in `poll.rs` computes percentages against the chosen base, using `data.frozenResults` and `data.entitledAtClose` (VOT-03). No new nodes. The `entitled` base requires the entitled snapshot to be present.

**Feasibility / Priority:** Small / Medium. Depends on VOT-01 and VOT-03.

### VOT-05: Named vs pseudoanonymous ballots via field suppression (formalize what exists)

**What OpenSlides does:** Polls are `named` (vote stores `user_id`) or `pseudoanonymous` (user references omitted, each vote gets an anonymous `user_token`); `poll.pseudoanonymize` nulls the linkage post-close. This is deletion-of-linkage, not cryptography. A separate cryptographic path exists but was never used in production.

**Why it fits:** The wiki already does exactly this: the `secret` flag drops `owner_id` and the backend tracks has-voted separately. So this is largely formalization plus the explicit post-close clear step, not a new capability, and it squarely fits the atproto pivot (records are public, so the voter-vote link must be broken deliberately).

**Model mapping:** Rename/promote the existing `secret` flag to `data.type` in `{named, pseudoanonymous}`. For pseudoanonymous, the trusted ballot service records the vote node without `owner_id` and stores the has-voted marker (a `user_token`) in a private table, never in a public atproto record. **Merged from VOT-06:** after tallies are frozen (VOT-03), the trusted service runs a post-close "clear" step that detaches the user-to-vote mapping while keeping the anonymous ballot count. This clear must happen server-side after tallies are frozen, else double-vote protection is lost mid-poll.

**Feasibility / Priority:** Medium / High. Do NOT adopt the cryptographic vote-decrypt path (OpenSlides' own README flags timing attacks and no production use). Ensure the has-voted marker lives only in the trusted service, not a public lexicon record. (One-vote enforcement itself, the old VOT-06, is already shipped via the `vote/vote` uniqueness constraint; only the anonymization clear step is folded in here.)

### VOT-07: Runoff rounds on a position (UI, structure already exists)

**What OpenSlides does:** Elections support multiple rounds (runoffs) by creating further polls on the same assignment; candidates are nominated off the roster.

**Why it fits:** Youth-org elections (e.g. electing a chair) frequently need a second round between top candidates. Structurally this already works: `vote/position` already parents multiple `vote/poll` children in the seed and `position.rs` already lists them. So the new work is UI, not capability.

**Model mapping:** Add a "create round 2" action that pre-populates a new poll with the top `vote/candidate` children from round 1 and labels rounds via `data.round`. Deciding who advances reads the frozen results of the prior round (VOT-03).

**Feasibility / Priority:** Small / Medium.

### VOT-08: Motion blocks, vote several policies together

**What OpenSlides does:** `motion_block` groups motions so an assembly adopts several at once as a block; best practice is to set recommendations for all before voting.

**Why it fits:** Landsmøder often adopt a batch of uncontroversial proposals in one vote to save floor time. The wiki's folders already group policies.

**Model mapping:** Model a block as a `wiki/folder` with `data.isBlock=true`. A single `vote/poll` parented to that folder decides all children; on acceptance, set each child's `data.status=accepted`. This requires a permission-template change: today the seed only allows polls under policy/change/position, so you must extend it to allow `vote/poll` under `wiki/folder`.

**Feasibility / Priority:** Medium / Low. Depends on MOT-01.

### VOT-09: Nominate candidates directly off the roster

**What OpenSlides does:** Candidates are proposed directly from the participant list (`option.content_object -> user`), with a `can_nominate_self` / `can_nominate_other` permission split.

**Why it fits:** The member app already has name-autocomplete for invitations; reusing it to nominate a roster member as a `vote/candidate` is a small, natural extension.

**Model mapping:** In `position.rs`, add a "nominate member" action reusing the member-autocomplete component, creating a `vote/candidate` whose `data` links the member `node_id` (and pulls their avatar/Bluesky picture). The `member` role already permits `vote/candidate` inserts under `vote/position`. Consider a self-vs-others permission distinction.

**Feasibility / Priority:** Small / Low.

### VOT-10: Multi-seat / list election voting methods

**What OpenSlides does:** Beyond single-option motion methods, board and delegation elections use multi-seat families: casting up to N votes across a candidate slate, per-candidate yes/no/abstain, optional cumulative voting, a global yes/no/abstain option, and `poll_candidate_list` for voting on whole lists.

**Why it fits:** VOT-01 only covers single-option motion methods. A youth-org board (*forretningsudvalg*) election with several seats and a limited number of votes per delegate cannot be expressed with For/Against/Blank or plain YNA. This is a real gap for elections.

**Model mapping:** Extend `data.method` on position ballots with a multi-seat variant plus a `data.maxVotesPerOption` cap and a `data.seats` count; accumulate per-candidate tallies over the existing `vote/candidate` model. Mark elected candidates via VOT-11.

**Feasibility / Priority:** Medium / Medium.

### VOT-11: Election result determination (elected marking, ties, seats)

**What OpenSlides does:** OpenSlides computes elected status from poll results against the number of seats and surfaces winners, handling the multi-seat case.

**Why it fits:** The wiki's election ideas let a position collect candidates and polls but nothing records *the outcome*, who won, how ties resolve, how many seats. Without it, an election has no recorded result.

**Model mapping:** After a position's poll closes (VOT-03), compute elected candidates from the frozen tallies against `data.seats` on the `vote/position`; stamp `data.elected=true` on winning `vote/candidate` nodes. Surface ties for an owner-driven tiebreak (a runoff via VOT-07).

**Feasibility / Priority:** Medium / Medium. Depends on VOT-03; pairs with VOT-10.

---

## Agenda & Speakers

### AGE-01: Agenda visibility levels (public / internal / hidden) with tree propagation

**What OpenSlides does:** `agenda_item.type` is `common`/`internal`/`hidden`; visibility propagates down the tree (a child of an internal parent is internal). The server computes `is_internal`/`is_hidden`.

**Why it fits:** The wiki hides certain mimes from the drawer but has no notion of "internal-only agenda entries" (a break, a board-only item). This gives the chair fine control over what the floor sees, distinct from drawer-hiding.

**Model mapping:** Add `data.visibility` (`public`/`internal`/`hidden`) on content nodes. Map `internal` -> visible only to the `owner` role, `hidden` -> owner-only. Propagate down the tree in the `program.rs` query (a child inherits the most restrictive ancestor visibility). Enforce via select permissions per role.

**Feasibility / Priority:** Medium / Medium. Tree-propagated visibility complicates select-permission evaluation; ensure it composes with the existing context permissions.

### AGE-02: Agenda numbering, duration estimates, and done-marking

**What OpenSlides does:** Agenda items get an auto `item_number`, an estimated `duration` (seconds), a moderator note, and a `closed` boolean marking an item done.

**Why it fits:** The program app already renders a numbered timeline with optional `data.time`. Adding a done-marker and duration estimate turns it into a real run-of-show the chair can drive.

**Model mapping:** Extend the program app: add `data.duration` and `data.closed` on content nodes; auto-number via the existing index/`get_index`. `closed` greys the item in `program.rs` and `admin.rs`. Duration enables a cumulative schedule display.

**Feasibility / Priority:** Small / Medium.

### AGE-03: Speaker states (waiting / current / finished) with speak-pause-end actions

**What OpenSlides does:** A speaker with no begin/end is waiting; begin set = current (single); both set = finished. Actions: speak, pause, unpause, end_speech; pause tracked via timestamps.

**Why it fits:** The wiki's `speak/list` + `speak/speak` already has current/next highlighting but **no** begin/end/pause timestamps. Adding them upgrades the queue to a proper debate manager and records who actually spoke, closing the "spoke list doesn't record who spoke" gap.

**Model mapping:** Add `data.beginTime`/`data.endTime`/`data.pause` on `speak/speak`. Owner actions in `speak.rs` set these; derive state from the timestamps. Keep timestamps server-authored to avoid clock drift.

**Feasibility / Priority:** Medium / High.

### AGE-04: Speech classification (pro / contra / contribution / point of order)

**What OpenSlides does:** `speech_state` is `contribution`/`pro`/`contra`/`intervention`/`interposed_question`; `point_of_order` flags a prioritized request inserted ahead of the queue, with optional ranked POO categories.

**Why it fits:** Balanced pro/contra debate and points of order are standard landsmøde procedure. This makes the speaker list procedurally meaningful, not just FIFO.

**Model mapping:** Add `data.speechType` (`pro`/`contra`/`contribution`/`pointOfOrder`) on `speak/speak`. The `speak.rs` queue renders pro/contra columns and inserts point-of-order entries at the front by priority. Members self-classify when joining the queue (already a member-insert action). Point-of-order queue-jumps must be owner-controlled to prevent abuse.

**Feasibility / Priority:** Medium / High. Depends on AGE-03.

### AGE-04b: Ranked point-of-order categories (refinement)

**What OpenSlides does:** `point_of_order_category` is its own configurable object with a `rank` that orders competing points of order, plus a meeting toggle to enable categories.

**Why it fits:** When several delegates raise different procedural points at once ("to the agenda" vs "to the vote"), the chair needs a defined precedence, not FIFO among POOs.

**Model mapping:** A per-context list of `{label, rank}` categories on the context `data`, and a `data.pooCategory` on `speak/speak` that drives front-of-queue insertion order. Refines AGE-04.

**Feasibility / Priority:** Small / Low. Depends on AGE-04.

### AGE-04c: Interposed questions and interventions as distinct request types

**What OpenSlides does:** Parliament mode adds `interposed_question` and `intervention` as first-class request-to-speak types with their own answer flow (a listener interjects a question the current speaker answers), distinct from pro/contra/POO.

**Why it fits:** The interposed clarifying-question flow is heavily used at youth-org debates and is separately modeled in OpenSlides. The wiki has `vote/question` but only bound to positions/files, not a live debate interjection.

**Model mapping:** Extend `data.speechType` with `interposedQuestion` and `intervention` on `speak/speak`. An interposed question is inserted next-to-current with a link to the speaker being questioned; the chair grants it. Can share the ephemeral channel (PLT-04) for the live "raise a question" ping before it becomes a `speak/speak` entry.

**Feasibility / Priority:** Medium / Low. Depends on AGE-03/AGE-04.

### AGE-05: Coupled speaking-time countdown with a warning threshold

**What OpenSlides does:** Speaking time is captured to the second; a list-of-speakers countdown auto-couples to `end_speech` with a warning-time threshold and a default time.

**Why it fits:** A visible, coupled countdown is exactly what the chair needs to keep debate on time. Note: the claim that `speak/list` already stores a `timeLimit` + last-start timestamp is **unconfirmed in the current `speak.rs`**, verify before building on it; the countdown rendering itself is new either way.

**Model mapping:** Add (or confirm) `speak/list` `data.timeLimit`. Render a live countdown on the screen/projector rail (`screen.rs`) that starts when a `speak/speak` enters `current` (AGE-03) and warns near zero. Anchor to the server-authored start timestamp to avoid client drift. Reuse the projector's live subscription.

**Feasibility / Priority:** Small / Medium. Depends on AGE-03.

### AGE-06: First-contribution highlighting and contribution counts

**What OpenSlides does:** Highlights a speaker's first contribution and tracks per-person speech counts with a filterable, exportable Contributions overview.

**Why it fits:** A youth org often wants to prioritize first-time or less-frequent speakers for inclusivity, a value-aligned feature. The member app already exports CSV.

**Model mapping:** Count a member's finished `speak/speak` entries across the context (AGE-03 gives finished turns); flag first-time speakers in `speak.rs`. Add a contributions view reusing the member app's paginated table with CSV export. Data derived from existing nodes.

**Feasibility / Priority:** Medium / Low. Depends on AGE-03.

### AGE-07: Speaker-list access settings (present-only, multiple lists, on-screen counts)

**What OpenSlides does:** Settings: only present users can be added; allow multiple concurrent speakers; how many finished/upcoming to show on the projector; show the total speaker count on the slide.

**Why it fits:** The wiki already supports multiple `speak/list` nodes and a read-only projector rail. These are cheap refinements the chair will want.

**Model mapping:** Add `data.*` toggles on `speak/list`: `presentOnly`, `showCount`, `amountNextOnScreen`. `presentOnly` gates member self-add against the presence flag (PAR-02). `screen.rs` reads `amountNextOnScreen` to limit the rail.

**Feasibility / Priority:** Small / Low. `presentOnly` depends on PAR-02.

---

## Projector & Autopilot

### PRJ-01: Projection queue (current / preview / history) with next/previous

**What OpenSlides does:** Each projector holds a current projection, a weighted preview queue (lowest weight = next), and history. Actions project/toggle/next/previous move content; a projection sits in exactly one queue.

**Why it fits:** The wiki's projector has only a single `active` relation (project one node, stop). A preview queue lets the chair line up the next agenda item and step through with next/previous, dramatically smoothing meeting flow.

**Model mapping:** Extend the context's projection relations from a single `active` to three ordered sets: `active`, `preview:N` (weighted), `history:N`. The admin app gains "add to preview" plus next/previous; next promotes the lowest-weight preview to `active` and pushes the old active to history. Reuses the existing relation + live-subscription mechanism.

**Feasibility / Priority:** Medium / High. If the full queue is too heavy for v1, keep a single "next" pointer.

### PRJ-02: Stable overlays, projector messages and a current-speaker chyron

**What OpenSlides does:** A projector may carry stable projections alongside the main slide: `projector_message` (HTML), countdowns, and an automatic lower-third chyron showing the current speaker (and their structure level).

**Why it fits:** The screen app already shows the active node plus a speaker rail. A current-speaker chyron and an ad-hoc message overlay ("Pause til 13:00") are high-visibility, low-cost additions.

**Model mapping:** Add stable overlay relations on the context: `message` (a short text/HTML blob) and `chyron` (auto-derived from the current `speak/speak` in AGE-03). `screen.rs` renders these over the hero pane. The chyron is a derived value recomputed on the live stream.

**Feasibility / Priority:** Small / Medium. Chyron depends on AGE-03.

### PRJ-03: Countdowns and count-up timers as projectable objects (with auto-coupling)

**What OpenSlides does:** `projector_countdown` has title/default_time/countdown_time/running; `default_time=0` counts UP. Two special auto-created countdowns couple to the speaker list and to polls (start when the poll opens, stop at close).

**Why it fits:** A visible countdown (speaking time, break) or count-up (total meeting elapsed) on the shared screen is a staple of well-run assemblies. The automatic poll-countdown coupling is a concrete, low-cost win worth calling out explicitly.

**Model mapping:** Add a lightweight countdown as `data` on the context (title/endTimestamp/running), projected as a stable overlay (PRJ-02). Auto-couple one to the speaker list (AGE-05) and one to an open poll: start it when the poll enters `started` (VOT-02) and stop at `finished`. No per-vote data needed.

**Feasibility / Priority:** Small / Medium. Depends on PRJ-02.

### PRJ-04: Autopilot, one consolidated follow-along view

**What OpenSlides does:** The Autopilot is a single easy-following screen that auto-shows whatever is currently projected in compressed form (current agenda item, current motion, current list of speakers, active vote/election, projector feed) so remote/mobile users never navigate between interfaces, and can interact (request to speak, classify speech, vote) from that one screen.

**Why it fits:** The wiki already has a follow app plus a separate vote app and speak app. Merging them into one autopilot is the single biggest UX win for mobile-first participants and matches the M3 "Assembly Canvas" redesign goal. This is a composition/UX task, not new backend capability, the components exist (`PollApp` already takes a projector prop; `SpeakApp` has a screen mode; the follow app tracks the active node).

**Model mapping:** Extend the follow app: track the context's `active` node and additionally embed (a) the active poll's ballot, (b) the current speaker list with a self-add / speak-request button, and (c) presenter focus. All three are existing components driven by existing live subscriptions on `active`/`focus`/screen-comments; the autopilot just composes them. Take care that it does not double-count against the existing follow app.

**Feasibility / Priority:** Medium / High. Depends on AGE-03 and PRJ-01. Adaptive layout must stay usable on a phone (aligns with the in-progress M3 redesign).

### PRJ-05: Multiple projectors and per-content-type defaults

**What OpenSlides does:** A meeting has one or more projectors plus a non-deletable reference projector; each content type has a `used_as_default_projector_for_<type>` assignment; styling (colors/header/logo/clock) is per projector.

**Why it fits:** Larger events use a main screen plus a side screen (e.g. speaker list on one, slide on another). Most youth-org meetings need only one, so this is a scale-flag feature.

**Model mapping:** Generalize the single `active` relation to named projector channels on the context (`active:main`, `active:side`). `screen.rs` takes a channel query param; default-per-type maps a mime to a channel. Keep one projector as the default.

**Feasibility / Priority:** Medium / Low. Opt-in; real complexity for little gain at youth-org scale.

---

## Participants & Permissions

### PAR-01: A layered role model beyond owner/member

**What OpenSlides does:** Meeting access is the union of a user's groups' permission strings; shipped groups include Admin, Delegates, Staff, Committees, and Guests. A group holds hierarchical `[domain].can_[action]` strings (parent implies child).

**Why it fits:** The wiki seed uses only `owner` and `member` roles, and the inventory flags "no role granularity" and "no delegation/secretary role". A landsmøde has a chair, a secretariat, delegates, and guests with genuinely different rights.

**Model mapping:** Generalize the permissions table's `role` column (today `member`/`owner`) to an open set of named roles per context, and store each member's role(s) in the members table. The permission rows already key on role, so this is mostly relaxing `role` to a string plus a role picker in the member app. Seed default roles (owner/chair, staff, delegate=member, guest). Permission evaluation must union multiple roles per member (today it is single-valued); keep the two-role default working for existing contexts.

**Feasibility / Priority:** Medium / High. Many downstream ideas (PAR-06, PLT-08, CHAT-01) depend on this.

### PAR-02: Presence (`present`) as a hard gate on vote eligibility and quorum

**What OpenSlides does:** Entitled voters = present AND in an entitled group (or delegated to). Presence is self-toggle or manager-set; it gates ballot casting and feeds quorum/roll-call.

**Why it fits:** Today eligibility = active member. Real assemblies require you to be present to vote, and quorum depends on the present count. Core to legitimate decisions. Note: the members table's `active` flag is currently used as attendance-ish, so introduce a distinct `present` to avoid conflation.

**Model mapping:** Add a `present` boolean on the members table (self-toggle in the follow/autopilot app, owner-set in the member app). Vote eligibility (and VOT-03's snapshot) becomes `active AND present`. Quorum = present count vs context `data.quorum`, shown in the admin console. Presence toggling at scale needs a cheap write path, consider a check-in step at meeting start; route the toggle through the ephemeral channel (PLT-04). Snapshot present-at-close, not just active (VOT-03).

**Feasibility / Priority:** Medium / High.

### PAR-03: Vote weight per member

**What OpenSlides does:** A per-participant `vote_weight` multiplier (min 0.000001), gated by a meeting toggle; each ballot contributes its weight rather than 1.

**Why it fits:** Delegate assemblies often weight votes by constituency size; a landsmøde may give local chapters weighted delegate votes.

**Model mapping:** Add `data.voteWeight` on the members table, honored only when the context sets `data.enableVoteWeight`. Tally computation in `poll.rs` sums weights instead of counts; the entitled snapshot (VOT-03) records each member's weight at close.

**Feasibility / Priority:** Medium / Low. Depends on VOT-03. Disable weight for pseudoanonymous polls (weight can deanonymize). Keep behind a toggle, often unnecessary at youth-org scale.

### PAR-04: Vote delegation / proxy voting

**What OpenSlides does:** Voting rights delegate to a proxy via `vote_delegated_to_id`; a present delegate casts on behalf of principals; optionally the delegating principal is barred; changes are recorded in History.

**Why it fits:** Delegates who cannot attend can assign a proxy, a common bylaw provision. The inventory notes "no delegation".

**Model mapping:** Add `data.delegatedTo` (a member `node_id`) on the members table. The trusted ballot service lets a present delegate cast extra ballots for their principals, barring the delegating principal. Eligibility extends VOT-05's has-voted logic.

**Feasibility / Priority:** Large / Low. Depends on PAR-02 and VOT-05. Proxy + secret ballot is a known hard case (the delegate could infer the principal's vote), restrict to named polls or anonymize carefully. Adds real complexity to the trusted vote service.

### PAR-05: Structure levels (delegations / factions) with color and result breakdown

**What OpenSlides does:** Named structure levels (with color) model delegations/factions; a participant can hold several; used for per-structure-level speaking-time pools and per-level result breakdowns.

**Why it fits:** Radikal Ungdom has local chapters (*lokalforeninger*) and factions. Tagging members by chapter enables per-chapter turnout and speaking-time fairness.

**Model mapping:** Add `data.structureLevels` (list of `{name,color}`) on the context and a `data.levels` tag list on members. `poll.rs` breaks results down by level; `speak.rs` can allocate time pools per level. Reuses the members table and existing tally rendering.

**Feasibility / Priority:** Large / Low. Per-level breakdown of a secret ballot risks deanonymization for small factions. Ship tagging + result breakdown first; speaking-time pools are heavy.

### PAR-06: Per-field access via read/write role lists

**What OpenSlides does:** Some capabilities bind to explicit group-id lists rather than a global permission, e.g. `motion_comment_section` `write_group_ids` (a "legal review" field readable only by a legal group) and chat read/write group lists.

**Why it fits:** Enables private annotation channels (a board-only note on a policy) without new mime types, extending the per-mime permission model to per-node granularity.

**Model mapping:** Allow a node's `data` to carry `readRoles`/`writeRoles` lists that the select/insert permission evaluation intersects with the viewer's role(s) (PAR-01). Apply first to `vote/comment` threads to create role-scoped comment sections (e.g. an internal board thread). Keep it additive and opt-in so it does not complicate the uniform context permissions table.

**Feasibility / Priority:** Medium / Medium. Depends on PAR-01.

### PAR-06b: Named parallel motion comment sections (structured fields)

**What OpenSlides does:** Motions carry several named comment sections (e.g. "Reason", "Legal review", "Board note"), each an ordered field with its own read/write groups, rendered inline in the motion detail and included in exports. This is the structural sibling of PAR-06's permission mechanism.

**Why it fits:** More than the wiki's single shared threaded comment: a small fixed set of role-scoped annotation *fields* on a policy. Gives the board a standing "Board assessment" field and the secretariat a "Procedural note" field per proposal.

**Model mapping:** A per-context list of comment-section definitions (label + read/write roles) plus `data` on `vote/policy` holding each section's rich text, gated by PAR-01 roles (reusing PAR-06's role intersection). Include sections in the ODT/minutes export (PLT-09).

**Feasibility / Priority:** Medium / Low. Depends on PAR-01/PAR-06.

### PAR-07: Invite expiry (DID onboarding is already the strategy)

**What OpenSlides does:** Central account management with SAML SSO, customizable invitation emails carrying credentials, and CSV/JSON import.

**Why it fits:** Invites already exist (`insertMembers`, the `accepted=false` flow), and the DID/Bluesky link is already the stated strategic direction, so the genuinely new part is narrow: invite *expiry* and surfacing stale invites, which the inventory flags ("no invitation expiry", invites that linger as `accepted=false` forever).

**Model mapping:** Add `data.invitedAt` / `data.expiresAt` on member rows and surface expired invites in the member app filter. Continue replacing SAML with atproto DID sign-in (already previewed via Bluesky linking) as the identity provider; keep the existing CSV import/export.

**Feasibility / Priority:** Medium / Medium.

### PAR-08: Personal notes and starred/favorite motions

**What OpenSlides does:** `personal_note` lets each participant privately annotate and star/favorite motions, visible only to that user.

**Why it fits:** The wiki has shared `vote/comment` and proposes role-scoped sections (PAR-06) but nothing private-per-user. Delegates preparing for a vote want private prep notes and a favorites shortlist to jump between the motions they care about during a fast-moving assembly.

**Model mapping:** Store personal notes and favorites keyed by (member, node) in the **trusted service or a private lexicon, never a public atproto record** (this is a hard privacy constraint worth designing in now, not discovering late). Surface a star toggle and a "my notes" panel in the vote/follow apps.

**Feasibility / Priority:** Medium / Low.

### PAR-09: Guest / public read-only access and shareable links

**What OpenSlides does:** It ships a Guests group and a public-access toggle for read-only followers.

**Why it fits:** No idea otherwise covers unauthenticated public follow-along, which is especially relevant given the atproto "records are public" pivot and the existing social/Bluesky share app. A public read-only projector/follow link is a natural feature for a landsmøde that wants to be watchable.

**Model mapping:** Seed a `guest` role (PAR-01) whose select permissions expose only public-visibility nodes (AGE-01). Add a shareable context link that opens the follow/screen app in a read-only `guest` session (no ballot/speak actions). The restrictor (PLT-01) must still hide internal/hidden and role-gated nodes from guests.

**Feasibility / Priority:** Medium / Medium. Depends on PAR-01 and AGE-01.

---

## Platform & Architecture

### PLT-01: Event-sourced record log + a restrictor/push service

**What OpenSlides does:** An append-only datastore (Create/Update/Delete/Restore events with monotonic positions) is the single source of truth; a separate autoupdate service turns the change stream into per-client, permission-filtered live pushes. Writes publish the set of modified fields to a stream; nothing polls.

**Why it fits:** This is the reference architecture for the wiki's custom Rust atproto backend. atproto records are already an append-only public log; a Rust push service that computes per-viewer permission-filtered diffs is exactly the autoupdate pattern, and cleanly replaces the current Hasura WebSocket subscriptions.

**Model mapping:** Treat atproto/lexicon records as the immutable event log (nodes as lexicon records). Build a Rust "autoupdate" service that watches the record firehose, applies a restrictor (the context permissions table + roles), and pushes minimal per-client diffs over one long-lived connection, replacing the per-relation Hasura subscriptions.

**Feasibility / Priority:** Large / High. This is the backbone of the whole atproto rewrite, not a feature. The restrictor must run server-side because atproto records are public by default.

### PLT-02: Declarative nested subscriptions (ModelRequest-style)

**What OpenSlides does:** A client subscribes with a ModelRequest tree (`{ids, collection, fields}` following relations); the server expands it to the exact fields to watch and diffs only changed, permitted ones per connection.

**Why it fits:** The wiki fires many discrete subscriptions (children, active, focus, screen-comments, poll votes). A single declarative nested subscription (a node plus its children plus comments plus authors) is more efficient and simpler for the autopilot view.

**Model mapping:** In the Rust backend, accept a typed request tree, e.g. `{node, children:{comments:{author}}, active, poll:{votes}}`. Expand to the watched record set; the restrictor filters; push minimal diffs. The Dioxus client declares what the current view needs in one request.

**Feasibility / Priority:** Large / Medium. Depends on PLT-01; build after basic push works.

### PLT-03: Derived/calculated fields recomputed on the change stream

**What OpenSlides does:** Fields like projector content and live poll tallies are registered as calculated fields, computed on demand and recomputed whenever underlying data changes, so projectors get live derived data through the same streaming mechanism.

**Why it fits:** The wiki already computes live tallies, `get_index` ordinals, the active-node view, and (proposed) the merged-final and chyron. Modeling these as reactive derived values keeps live views correct without storing redundant state, fitting append-only records.

**Model mapping:** In the autoupdate service, register derived computations: live poll tally (from `vote/vote` children + poll state), the projected-node view, `get_index` ordinals, the current-speaker chyron (PRJ-02), and merged-final (MOT-05). Recompute on the same stream that drives pushes; bound recomputation cost.

**Feasibility / Priority:** Medium / Medium. Depends on PLT-01.

### PLT-04: An ICC-style ephemeral channel service

**What OpenSlides does:** An ICC service handles non-persisted real-time messaging (notify, chat, applause with a present-user count) targeting a meeting, a user, or a channel id, keeping transient traffic out of the durable datastore.

**Why it fits:** The atproto plan says "ephemeral state stays server-side". Presence toggles (PAR-02), applause/reactions, and transient pings should NOT become permanent public atproto records. A separate ephemeral channel keeps the record store clean.

**Model mapping:** Add a Rust ephemeral channel service (WS) for presence, reactions/applause, and "raise hand" pings, addressed by context/member/channel. These never become nodes/lexicon records. Presence (PAR-02) and the applause feature route through it. (Two real-time paths, durable diffs + ephemeral, are the cost.)

**Feasibility / Priority:** Medium / Medium.

### CHAT-01: In-meeting chat groups with read/write role lists

**What OpenSlides does:** `chat_group` + `chat_message` are real objects: each group's `read_group_ids`/`write_group_ids` restrict who sees and posts, gated by a "Can manage chat" permission. (PLT-04 lists chat only as ephemeral transport, this is the user-facing feature.)

**Why it fits:** A landsmøde wants a delegates-only coordination channel, a staff/secretariat channel, and a guest Q&A channel, all kept separate from the durable node tree. This is a small composition on top of already-proposed primitives but deserves to be an explicit deliverable.

**Model mapping:** Named channels whose read/write role lists intersect with the viewer's role(s) (PAR-01), transported over the ephemeral service (PLT-04) so messages do not become public atproto records. A "manage chat" permission controls channel creation.

**Feasibility / Priority:** Medium / Medium. Depends on PAR-01 and PLT-04.

### PLT-05: Change history / audit trail with restore

**What OpenSlides does:** A restorable History records every action; deleted objects can be restored by id; motion and delegation timestamps are tracked. Gated by a history/manage permission. (The OS4 permission name is not the OS3 `can_see_history`, so gate on a history/manage permission rather than a specific string.)

**Why it fits:** The inventory flags "no audit log". For a governance tool this matters for disputes and transparency, and an append-only event log makes history nearly free.

**Model mapping:** Because PLT-01 makes the record store append-only, history is intrinsic: every node version is a record. Surface a per-node history view and an owner-only "restore deleted node" action gated by a history-role permission. No separate audit table. Public atproto history could expose who edited what, so gate the UI and decide what belongs in public records vs the trusted service.

**Feasibility / Priority:** Medium / Medium. Depends on PLT-01.

### PLT-06: Auth ticket = short-lived JWT + httpOnly refresh cookie

**What OpenSlides does:** Login issues a short-lived JWT access token plus an httpOnly `refreshId` cookie (mitigating XSS+CSRF); an internal `authenticate` endpoint returns `{userId, sessionId}`; a published logout event live-closes subscription streams across services.

**Why it fits:** The wiki is moving off NHost auth to a custom Rust backend with DID identity. This is a proven, secure session model, and logout-closes-streams pairs directly with PLT-01's push service.

**Model mapping:** In the Rust backend, issue a short-lived JWT (bound to the atproto DID) + httpOnly refresh cookie; an internal `authenticate` endpoint returns `{did, sessionId}` to the autoupdate and vote services; a logout event closes the member's live subscriptions. Reconcile with atproto OAuth/DID session semantics rather than inventing a fully bespoke scheme.

**Feasibility / Priority:** Medium / Medium. Depends on PLT-01.

### PLT-07: Optimistic-concurrency writes to guard against lost updates

**What OpenSlides does:** The datastore uses optimistic concurrency control: writes carry `locked_fields` (the position at which data was read); an intervening write fails with `ModelLocked` and the client retries. Reader and writer are split.

**Why it fits:** The inventory flags "no real-time edit sync: concurrent edits not merged (last-write-wins)". OCC at least detects and rejects lost updates instead of silently clobbering, a meaningful safety improvement for co-edited policies.

**Model mapping:** Attach a version/position to each node; the editor's autosave sends the version it read; the Rust writer rejects a stale write and the client surfaces a "reloaded, please reapply" snackbar. Cheap safety net short of full CRDT merging (users may find retry friction, but it beats silent last-write-wins).

**Feasibility / Priority:** Medium / Low.

### PLT-08: Stream-fed, restrictor-filtered full-text search

**What OpenSlides does:** A search index is filled from the change stream, parses HTML fields, and always post-filters results through the restrictor; only "basic" always-visible fields are indexed.

**Why it fits:** The wiki already has Postgres full-text search over extracted Slate text (a generated column), so FTS itself is not new. The novelty is feeding it from the change stream and enforcing the restrictor so results stay permission-correct in the new architecture.

**Model mapping:** Feed the Rust search index from the PLT-01 change stream (reusing the existing plain-text extraction). Post-filter hits through the context permission restrictor + roles before returning, so role-gated nodes (PAR-06) never surface in search.

**Feasibility / Priority:** Medium / Low. Depends on PLT-01.

### PLT-09: Richer exports (minutes, results, ballot papers, diffs)

**What OpenSlides does:** One-click PDF exports for minutes, motions (with the four text views incl. diff, ToC, configurable columns, comment sections), elections with results, analog ballot papers, and participant access data, plus CSV/XLSX/ZIP.

**Why it fits:** The wiki already has ODT (content) and member CSV export, so those are not new. The new, valuable additions are an agenda/minutes export and an official results record, which a landsmøde needs for the record.

**Model mapping:** Extend the existing ODT/CSV pipeline: add an agenda/minutes export (program tree + statuses), a per-poll results export (frozen results + turnout from VOT-03), printable ballot papers for analog polls (VOT-13), and a policy export including amendment diffs (MOT-06). Ship results + minutes first; diff export depends on MOT-04/06.

**Feasibility / Priority:** Medium / Medium. Depends on VOT-03.

### PLT-10: Per-assembly custom translation overrides

**What OpenSlides does:** Multilingual UI with per-meeting custom translations that override any phrase.

**Why it fits:** The wiki ships fixed en/da bundles. A youth org uses its own jargon (*landsmøde*, *ordstyrer*, *forslag*). Per-context term overrides let each assembly speak its own language without a code change.

**Model mapping:** Store a `data.translations` override map on the context; the i18n layer checks context overrides before falling back to the shipped en/da bundle. Owner-editable. A small addition to the existing i18n system. Keep the map small and owner-managed to avoid drift.

**Feasibility / Priority:** Small / Low.

### VOT-13: Analog (paper) poll type and printable ballot papers

**What OpenSlides does:** `poll.type` is `analog`/`named`/`pseudoanonymous`. Analog polls have results entered by hand and printable ballot papers via PDF, and forbid the `entitled` 100%-base (there is no per-user electronic record to count).

**Why it fits:** VOT-05 covered only named vs pseudoanonymous and dropped `analog`. Analog polls matter for hybrid assemblies where some vote on paper, and they explain why `entitled` is not a valid base for those polls.

**Model mapping:** Add `analog` to `data.type` on `vote/poll`. For analog, the owner enters `data.frozenResults` directly (no `vote/vote` inserts), and `poll.rs` disallows the `entitled` base (VOT-04). Printable ballot papers ride the export pipeline (PLT-09).

**Feasibility / Priority:** Small / Low.

### CTX-01: A per-context meeting-settings surface

**What OpenSlides does:** OpenSlides centralizes meeting configuration in a settings screen (workflow, quorum, voting toggles, translations, defaults).

**Why it fits:** Nearly every High-priority idea above adds a new context `data.*` field (workflow, quorum, minSupporters, enableVoteWeight, structureLevels, translations, base defaults). Without an admin UI to edit them per context, those ideas have **no configuration entry point**. This is the connective tissue the catalog otherwise omits.

**Model mapping:** Add a settings panel in the admin app that reads/writes the context node's `data.*` config fields, owner-only via the update permission. Group fields by theme (motions, voting, speakers, projector, i18n).

**Feasibility / Priority:** Medium / High. Effectively a prerequisite for shipping the High-priority `data.*` features usefully.

### CTX-02: Meeting clone and archive for annual reuse

**What OpenSlides does:** `meeting.clone` duplicates a meeting as a template and `meeting.archive` marks a past meeting read-only, so an annual assembly's structure, groups, workflows, and permissions are reused rather than rebuilt.

**Why it fits:** Radikal Ungdom runs a landsmøde every year. Cloning last year's event (agenda skeleton, roles, workflow, translations) is a large practical time-saver, and archiving keeps the historical assembly immutable and out of the active list.

**Model mapping:** A deep-copy of a `wiki/event` subtree (structure only, not ballots/votes) plus a context `data.archived` flag that switches the permission evaluation to read-only. Reuses the node-copy machinery.

**Feasibility / Priority:** Medium / Medium.

---

## Prioritized shortlist

The near-term roadmap: High-priority ideas that are Small or Medium feasibility, ordered so prerequisites come first.

| Order | Idea | Theme | Feasibility | Why now |
|---|---|---|---|---|
| 1 | CTX-01 Per-context settings surface | Platform | Medium | The config entry point every `data.*` feature below needs |
| 2 | MOT-01 Motion workflow / `data.status` | Motions | Medium | The #1 named gap; makes the proposal lifecycle visible |
| 3 | VOT-02 Poll lifecycle state machine | Voting | Medium | Chair-controlled result reveal; replaces the binary open/closed flag |
| 4 | VOT-01 Poll methods Y/YN/YNA/N | Voting | Medium | Standardizes motion voting; unlocks consistent result math |
| 5 | VOT-03 Freeze results + entitled snapshot | Voting | Medium | Trustworthy immutable governance record (turnout stops drifting) |
| 6 | PAR-02 Presence gate + quorum | Participants | Medium | Legitimacy: present-to-vote and a real quorum count |
| 7 | PAR-01 Layered role model | Participants | Medium | Chair/staff/delegate/guest; unblocks PAR-06, PAR-09, CHAT-01 |
| 8 | AGE-03 Speaker begin/end/pause states | Speakers | Medium | Real debate manager; records who actually spoke |
| 9 | AGE-04 Speech classification (pro/contra/POO) | Speakers | Medium | Procedurally meaningful speaker list |
| 10 | PRJ-01 Projection preview queue | Projector | Medium | Line up the next item; step through with next/previous |
| 11 | PRJ-04 Autopilot follow-along | Projector | Medium | Biggest mobile UX win; composes existing components; fits M3 canvas |
| 12 | VOT-05 Named/pseudoanonymous formalization | Voting | Medium | Formalizes the secret flag for the atproto public-records pivot |

**Bigger bets (not near-term, but strategic):** PLT-01 (append-only datastore + restrictor/push service) is the backbone of the entire atproto rewrite, not a feature, and everything in the Platform section (PLT-02/03/05/06/08) depends on it. Treat PLT-01 + PLT-06 (auth) as the platform milestone that the feature work eventually migrates onto. MOT-04/05/06 (line/block-anchored amendments, auto-merge, four views) are a large but high-value front-end investment worth scheduling once workflows (MOT-01) land. CTX-02 (clone/archive) pays off before each annual landsmøde.

## Caveats & fit

Not everything in OpenSlides belongs in RadikalWiki. Being honest about the misfits:

- **Scale features add cost without payoff.** Multiple projectors (PRJ-05), vote weight (PAR-03), structure-level speaking-time pools (PAR-05), and proxy voting (PAR-04) exist for large delegate parliaments. A youth-org landsmøde rarely needs them; ship them opt-in, behind a context toggle, or not at all until asked.
- **The privacy pivot cuts against several designs.** atproto records are public by default. That means: the has-voted marker and the pseudoanonymize clear step (VOT-05) must live only in the trusted service, never a public lexicon; personal notes and favorites (PAR-08) must not become public records; per-level and weighted result breakdowns (PAR-03, PAR-05) can deanonymize small factions on secret ballots; and public change history (PLT-05) could expose who authored/edited what. The restrictor (PLT-01) is load-bearing precisely because it is the only thing standing between role-gated content and a public record store. Do NOT adopt OpenSlides' cryptographic vote-decrypt path: its own README flags timing attacks and it was never used in production.
- **Append-only clashes with delete-and-reset semantics.** OpenSlides' `reset` deletes votes and `restore` un-deletes objects. On an append-only atproto log, model a poll reset as a *new* poll and model deletion as a tombstone record, not an in-place mutation.
- **Complexity is a real cost for a small org.** Full customizable-workflow editors, ten-state workflows, ranked point-of-order categories, and multi-seat cumulative voting are powerful but heavy. Ship the fixed 3-4 state workflow, the fixed pro/contra/POO set, and plain YNA first; add configurability only when an assembly actually asks for it. Most of the value is in the first, simplest version of each idea.
- **Some "ideas" are already done.** Amendments-of-amendments, ordinal labeling, one-vote enforcement, immutable votes, hidden results, secret ballots, ODT/CSV export, and full-text search already ship. Do not rebuild them; the work is surfacing and formalizing what exists (MOT-09, VOT-06).

## References

Concrete OpenSlides sources used and verified for this document:

- **Data model:** `OpenSlides/openslides-backend` `models.yml` (poll `pollmethod` Y/YN/YNA/N, poll `state` created/started/finished/published, `poll.type` analog/named/pseudoanonymous, `entitled_users_at_stop`, `onehundred_percent_base` including N, `motion_state` flags incl. `merge_amendment_into_final` do_merge/do_not_merge/undefined and `set_number`, `vote_weight`, `vote_delegated_to_id`, `structure_level`, `agenda_item.type`).
- **Actions:** `OpenSlides/openslides-backend` Actions-Overview (`motion.create_forwarded`, `poll.start`/`stop`/`publish`/`reset`, `poll.pseudoanonymize`, speaker `speak`/`pause`/`unpause`/`end_speech`, projection `project`/`next`/`previous`, `meeting.clone`/`archive`).
- **Feature docs:** the Motions, Voting, Projection/Projector, Countdowns, Datastore-Service, Autoupdate-Service GitHub wikis (workflows, change recommendations, four text views, motion blocks/categories, supporters, projection queue and overlays, calculated fields, `locked_fields`/`ModelLocked` OCC).
- **Service READMEs:** `openslides-icc-service` (notify/chat/applause with present-user count), `openslides-auth-service` (short-lived JWT + httpOnly `refreshId` cookie, `authenticate` endpoint).
- **Product pages:** openslides.com/en/auto-pilot (Autopilot), openslides.com/en/functions, openslides.com/en/other-functions, openslides.com/en/committees (committees, forwarding, guests, chat groups, custom translations, exports).

Corrections applied from source verification: the 100%-base set includes `N` and has no separate `entitled_present` (present is folded into `entitled`); OpenSlides seeds "a simple and a complex workflow" without asserting an exact state count; the OS4 history permission is a history/manage permission, not the OS3 `can_see_history` string; and OS4 amendments are driven by paragraph/text modes as well as line ranges, so block-level anchoring is the right adaptation for Slate content.
