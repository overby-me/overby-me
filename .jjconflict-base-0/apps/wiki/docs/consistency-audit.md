# Design & Functionality Inconsistencies

## Intro

This audit covers the whole wiki-dioxus application (the single nodes tree and all `?app=` views: projection/screen, voting, speakers, comments, members, editor, sort, navigation, admin, plus the M3 design system and i18n). Each area was scanned by a dedicated finder, then every candidate was adversarially re-verified against the code so that false positives were removed before inclusion. After deduplication and merging of near-duplicate findings, the confirmed count is: 13 High, 18 Medium, 10 Low.

The single most important theme, and the one the user pointed at, is a **node-type projection/action asymmetry**: nodes rendered by `FolderApp` (wiki/group, wiki/event, wiki/folder) and by the ballot apps get a much thinner action set than nodes rendered by `ContentApp` (wiki/document, vote/candidate and the vote content types), even though the underlying projection, sharing, delete, edit, and comments mechanisms are not mime-gated on the backend.

## Top inconsistencies

### Groups/events/folders cannot be projected from their own view (documents can)

What: Only nodes rendered by `ContentApp` (wiki/document, vote/candidate) expose the "Project on screen" action. A wiki/group, wiki/event, or wiki/folder viewed as its own context renders via `FolderApp`, which has no projection button, so a chair cannot project a group listing or event description to the room, even though a document in that same group can be projected. The projection primitive itself (`set_active_relation`) is not mime-gated and works for any node.

Evidence: `src/components/content.rs:189-209` (projectScreen button, gated on `can_manage`, only in ContentApp); `src/components/loader.rs:210-211` (wiki/group|wiki/event route to FolderApp); `src/components/folder.rs:116-207` (FolderApp ToolSheet has only copy-link, export, sort, lock, paste: no projection); `src/graphql.rs:741-762` (`set_active_relation` has no mime validation).

Why it matters: This is the core workflow break the user flagged. Owners cannot present the very containers (groups/events) that host the room and its projector.

Fix: Add a conditional `projectScreen` action to FolderApp's ToolSheet for wiki/group and wiki/event (and wiki/folder), computing `node_context` as `context_id || node.id` like ContentApp (`content.rs:65-69`) and gating on `can_manage = is_owner || is_context_owner`.

### Projected groups/events render full editing chrome instead of a read-only projector view

What: When a group/event/folder is set active and shown on `?app=screen`, `MimeLoader` routes it to `FolderApp` with its full toolbar, add buttons, and child list. wiki/document and vote/candidate have explicit `if projector` branches that render a lean read-only view, and vote/poll takes a `projector` flag to hide the ballot; group/event/folder have no such branch.

Evidence: `src/components/loader.rs:194` (wiki/document `if projector`), `loader.rs:221` (vote/candidate `if projector`), `loader.rs:237` (vote/poll gets projector flag); `loader.rs:210-211` (wiki/group|event unconditionally FolderApp, no projector branch); `poll.rs:341` (`!projector` gate).

Why it matters: A projected group/event is unusable as a room-facing screen: it shows edit chrome and interactive controls instead of a clean presentation.

Fix: Add projector-aware branching in MimeLoader for wiki/folder/group/event (like `loader.rs:194`), or pass a `projector` flag into FolderApp and suppress the ToolSheet, add buttons, and child filtering when `projector==true`; optionally project only the node's Slate description as a single card.

### Owners cannot delete folders/groups/events from the UI (documents and files can)

What: wiki/document and wiki/file expose a delete button gated on `can_manage && !segments.is_empty()`, but wiki/folder/group/event (FolderApp) and vote/poll (PollApp) have no delete UI at all, so owners cannot delete these node types through the app.

Evidence: `src/components/content.rs:255-261` (document delete), `src/components/file.rs:159-165` (file delete); `src/components/folder.rs:116-207` (FolderApp: no delete), `src/components/vote/poll.rs:326-546` (PollApp: only stop-poll, no delete).

Why it matters: The node permission model lets owners delete any node, but the UI silently omits the affordance for the most container-like types, forcing backend-only deletion or dead ends.

Fix: Add a delete button plus confirm dialog to FolderApp (group/event/folder) following `content.rs:255-313`, gated on `can_manage && !parent_path.is_empty()`, calling `delete_node` and `delete_node_members`; reuse `content.confirmDelete` / `common.delete`.

### Context owners cannot manage speaker lists despite having permission

What: In `speak.rs`, `is_owner` is computed from `node.owner_id` alone, but the adjacent comment says "The context owner may manage the list(s)". All admin/add gates use only `is_owner`, so context owners see no admin panel or add button even though speak/list is owner-role manageable. `member.rs` already uses the correct dual check.

Evidence: `src/components/speak.rs:31-32` (comment vs code mismatch), `speak.rs:44` (can_add), `speak.rs:435` (admin panel gate), `speak.rs:361`; correct pattern at `src/components/member.rs:21-22` (`is_owner || node.is_context_owner`).

Why it matters: The person actually running the assembly (the context owner) is locked out of managing speaker queues, which is a functional permission bug, not just a styling gap.

Fix: Add `let is_context_owner = node.is_context_owner.unwrap_or(false);` and use `can_manage = is_owner || is_context_owner` for the gates at lines 44, 361, and 435, matching member.rs.

### Speaker-queue and sort mutations discard errors, faking success

What: Every queue mutation in `speak.rs` (move up/down, remove, next speaker, open/close, clear, join, timer) uses `let _ = graphql::...().await;` and unconditionally fires `refresh += 1`, and `sort.rs`'s save loop ignores each `update_node` result then shows a success snackbar and navigates away. Users see apparent success even when the backend rejected the change. `create_speaker_list`, `vote/poll.rs`, and `comments.rs` all show proper snackbar errors.

Evidence: `src/components/speak.rs:377,399,418,458,485,509,607,649` (silent `let _ =`); contrast `speak.rs:116-117` and `src/components/vote/poll.rs:352-366`; `src/components/sort.rs:44-52` (loop ignores results, then `show_snackbar("sort.saveSorting")` unconditionally).

Why it matters: A failed reorder or queue change leaves the server unchanged while the UI claims it worked, and the live subscription will not refire on a failed mutation, so the user waits indefinitely.

Fix: Convert these to `match` on the result; on `Err`, log and `show_snackbar(&t("error.somethingWentWrong"))`. For `sort.rs`, collect results and only show success / navigate if all updates succeeded.

### AddChangeButton fails silently without closing its dialog

What: In `AddChangeButton`, a failed amendment insert leaves the dialog open with no error, because the submit handler only branches on `is_ok()` with no else. `StartPollButton` in the same subsystem closes the dialog and shows a snackbar on failure.

Evidence: `src/components/vote/position.rs:307` (`if insert_node(...).is_ok()` with no else), `position.rs:326-343` (dialog never closed in submit); contrast `src/components/vote/poll.rs:740-741`.

Why it matters: The user gets no feedback and may repeatedly resubmit a failing amendment.

Fix: Convert to `match`; in the `Err(e)` branch call `open.set(false)` and `show_snackbar(&e)` (add the `show_snackbar` import), matching StartPollButton at poll.rs:739-742.

### Comment UI is mounted on node types that cannot accept comments

What: `CommentSection` is rendered for wiki/document, wiki/file, vote/position, and vote/candidate, but the permission template only allows vote/comment under vote/policy and vote/change. The composer is correctly gated on `can_comment`, so the section shows a "Comments"/"No comments yet" state with no way to post, implying comments exist-but-empty rather than unsupported.

Evidence: `src/components/loader.rs:201-204` (wiki/document), `src/components/file.rs:265-268` (wiki/file), `src/components/vote/position.rs:262` (vote/position), `src/components/loader.rs:229-232` (vote/candidate); permission rule at `src/graphql.rs:1808`; composer gate at `src/components/comments.rs:161`.

Why it matters: A false affordance across four common node types; users (and guests) cannot tell "no comments yet" from "comments not available here".

Fix: Mount `CommentSection` only for node types the permission matrix supports (vote/policy, vote/change), or when `can_comment` is false due to node type, render a distinct "Comments not available for this item" state (`comments.rs:151-227`).

### Delete-permission asymmetry: question authors can delete, comment authors cannot

What: vote/question and vote/comment have identical permission rows (member-insertable, `mutable_row=false`), yet the UI shows author/context-owner delete buttons for questions but no delete affordance at all for comments. The permission flags are fetched for both.

Evidence: `src/graphql.rs:1807` (vote/question) and `src/graphql.rs:1808` (vote/comment) identical immutability; `src/components/vote/position.rs:169-170,186-203` (question `can_del` and delete button); `src/components/comments.rs:332-408` (comment thread has no delete).

Why it matters: Identical permission rows behave differently, so users cannot understand why they may delete a question they authored but not a comment they authored.

Fix: Make the two consistent: either add a `can_del = is_owner || is_context_owner` delete affordance to comments, or remove the question delete buttons, or make both mime types mutable and author-gated. Pick one model and apply it to both.

### Missing i18n keys make ballot validation errors show raw keys

What: `poll.rs` calls three vote-validation keys that exist in neither EN nor DA: `vote.blankOnlyAlone`, `vote.selectAtLeast` (interpolated), and `vote.selectAtMost` (interpolated, used twice). When a voter violates min/max or combines Blank with other choices, the literal key string is shown instead of a message.

Evidence: `src/components/vote/poll.rs:268` (`vote.blankOnlyAlone`), `poll.rs:273` (`vote.selectAtLeast`), `poll.rs:277` and `poll.rs:575` (`vote.selectAtMost`); missing from EN (`src/i18n.rs:289-322`) and DA (`src/i18n.rs:700-732`).

Why it matters: The exact moment a voter needs guidance (they picked too few/too many options), they see a developer key in both languages.

Fix: Add `blankOnlyAlone`, `selectAtLeast` (`{{count}}`), and `selectAtMost` (`{{count}}`) to both the EN and DA vote sections of `src/i18n.rs`.

### Raw px spacing bypasses the M3 spacing token system

What: The design system defines `--md-sys-spacing-*` tokens, but the stylesheet overwhelmingly uses raw px for padding/margin (about 213 raw-px declarations vs 25 using `var()`), including on-scale and off-scale values, plus a runtime `padding-left` computed in raw px in the drawer.

Evidence: `assets/style.css:410` (`padding: 20px`), `:426` (`margin-bottom: 10px`), `:1003` (`.drawer-context-bar`), `:1057` (`.card margin-bottom: 16px`); `src/components/layout/drawer.rs:213` (`format!("padding-left: {}px;", 12 + depth * 14)`).

Why it matters: Spacing cannot be tuned centrally through tokens, so density/adaptivity changes require touching hundreds of literals and off-scale values drift.

Fix: Replace on-scale px with `--md-sys-spacing-1..6`, add tokens (or intentionally document) for off-scale values, and drive the drawer indentation from a CSS custom property or a token base unit.

### Only .btn-primary has a disabled style; other variants give no disabled feedback

What: `:disabled` styling is defined only for `.btn-primary`. `.btn-secondary`, `.btn-tonal`, `.btn-outlined`, `.btn-text`, `.btn-icon`, and `.btn-icon.add-action` have hover/active but no disabled rule, so `disabled=true` buttons look identical to enabled ones. These variants are used with `disabled` in real code.

Evidence: `assets/style.css:1719` (`.btn-primary:disabled`), `:1737-1750` (`.btn-tonal` no disabled), `:1758-1770` (`.btn-outlined` no disabled), `:1789-1847` (`.btn-icon` no disabled); real usages: `editor.rs:506`, `profile.rs:266`, `speak.rs:500`, `table.rs:105,115`.

Why it matters: Users cannot tell disabled controls from enabled ones, so they click dead buttons (compounding the silent-failure findings above).

Fix: Add a shared `:disabled` rule to all button variants using the M3 disabled pattern (12% surface background, 38% on-surface text, `cursor: not-allowed`), matching `.btn-primary:disabled`.

## By area

### Projection

- Focus/section projector control vanishes for non-heading types: projecting wiki/file or vote/poll from the admin agenda hides the focus card since headings only come from a document's `data.content`. Where: `src/components/admin.rs:63-79,213`, `src/components/content.rs:435-454`. Fix: restrict agenda items to heading-bearing types, or show a disabled/informational focus card when there are no headings.
- Comments-on-screen toggle only exists in ContentApp: `ScreenApp` will render comments for any active node, but only wiki/document and vote/candidate expose the toggle, and `set_screen_comments` is unvalidated. Where: `src/components/content.rs:211-242`, `src/components/screen.rs:126-135`, `src/graphql.rs:828-846`. Fix: add the toggle to projectable FolderApp/ballot types, or restrict the ScreenApp comment path to ContentApp node types.
- Screen-comments toggle also missing from FolderApp/vote-poll: groups/events render CommentSection unconditionally below their listing with no owner suppression control. Where: `src/components/content.rs:211-242`, `src/components/folder.rs:298-303`, `src/components/screen.rs:124-134`. Fix: add the `set_screen_comments` toggle to FolderApp for group/event, or document the omission.
- FAB vs docked ToolSheet is not the cause of the action gap (partial): ToolSheet correctly renders identical children docked and modal; the group/event vs document difference is a node-type action-set difference resolved by the FolderApp action fixes above. Where: `src/widgets/tool_sheet.rs:79-114`, `src/components/folder.rs:116-207`. Fix: none for ToolSheet; fixing the FolderApp action set makes the docked sheet correct automatically.

### Voting

- Add-question fails silently: on insert failure PositionApp shows no error and has already cleared the input before the await, so the typed text is lost. Where: `src/components/vote/position.rs:56,68-70`; contrast `poll.rs:365`. Fix: `match` the insert, show a snackbar on error, and only clear `q_text` on success (or save/restore it).
- Amendment capability asymmetry (by design): PolicyApp offers `AddChangeButton` but PositionApp does not, because the backend only allows vote/change under vote/policy/change/file, not vote/position. Where: `src/components/vote/policy.rs:51`, `src/components/vote/position.rs:75-264`, `src/graphql.rs:1810-1814`. Fix: document the intended limit in a comment near PolicyApp; no UI change needed.
- Add-candidate affordance missing (by design, incomplete): PositionApp has inline add-question but no add-candidate; both are member-insertable, but candidates need a photo upload not yet built (PLAN.md:83). Where: `src/graphql.rs:1796,1807`, `src/components/vote/position.rs:82-136,211-227`. Fix: add an inline add-candidate form once photo upload exists, or document the editor-only path meanwhile.
- Poll close is context-owner-only but the label does not say so (partial): the `Stop poll` gate correctly matches the owner-role permission, but the poll creator is not told only the context owner can close it. Where: `src/components/vote/poll.rs:341,344`, `src/graphql.rs:1803-1806`. Fix: clarify the aria-label/title or defer pending user testing.

### Speakers

- No confirmation on destructive queue actions: remove-speaker and clear-queue delete with one click and no dialog, unlike member removal. Where: `src/components/speak.rs:407-425,498-517`; pattern at `src/components/member.rs:387-417`. Fix: add a confirm Dialog with a confirmation-state variable before remove/clear.
- Join-queue insert ignores errors: clicking Talk/Question ignores the `insert_node` result, so a failed join leaves the button active but nothing in the queue. Where: `src/components/speak.rs:607-622`; contrast `speak.rs:116-118`. Fix: `match` the insert, snackbar on error, and disable the button during the await.
- No pending/disabled state on move/remove buttons: async mutations have no in-flight disabling, so rapid clicks spawn overlapping mutations. Where: `src/components/speak.rs:362-405,407-425,500`. Fix: add a `pending` signal to disable buttons until `refresh` completes, reusing the AddSpeakerListButton busy pattern.
- No optimistic feedback for join/move like comments have: comments show optimistic rows with rollback; speak join/move give no feedback until the next subscription refresh. Where: `src/components/comments.rs:26-48,450,500-504`, `src/components/speak.rs:557-650,370-430`. Fix: at minimum add error snackbars (optimistic rendering optional).

### Comments

- Asymmetric CommentSection among FolderApp types: wiki/group and wiki/event render CommentSection but wiki/folder does not, though all three use FolderApp and none can actually accept vote/comment. Where: `src/components/folder.rs:298-302,40-87`, `src/graphql.rs:1808`. Fix: treat all three consistently, and drop the false affordance if comments are unsupported.
- Empty state does not distinguish "none yet" from "unsupported": the container/heading/empty state always render even where `can_comment` is false by node type. Where: `src/components/comments.rs:151-227,161,201-208`. Fix: render a "Comments not available for this item" state when the node type cannot accept comments.

### Members

- Promote/demote owner has no confirmation while remove does: ownership changes execute immediately, but the less-sensitive remove requires a dialog. Where: `src/components/member.rs:557-569`; pattern at `member.rs:388-420`; labels at `src/i18n.rs:454-455`. Fix: add a confirm dialog for promote/demote, especially when demoting the last owner.
- Edit-member save button has no disabled/validation: save can persist a member with both name and email cleared, unlike the invite button which disables on empty input. Where: `src/components/member.rs:367,86-92`; contrast `member.rs:280`. Fix: add `disabled: edit_name.trim().is_empty() && edit_email.trim().is_empty()`.
- Single-invite success message reuses a button label: inviting shows `invite.invite` ("Invite") instead of an action-completion message, while bulk import correctly uses `invite.imported`. Where: `src/components/member.rs:259,294,336`, `src/i18n.rs:429`. Fix: add an `invite.sent` key and use it at 259 and 294.

### Content/Editor

- FileApp persists and deletes `node.members` but never displays them: files can take authors (not excluded in `node_takes_authors`) and their members are cleaned up on delete, yet FileApp shows no author chips like ContentApp does. Where: `src/components/file.rs:193`, `src/components/content.rs:316-331`, `src/components/editor.rs:103-107`. Fix: either exclude wiki/file from `node_takes_authors`, or render author chips in FileApp.
- Empty rich content in ContentApp is a bare muted `<p>`: ContentApp always renders SlateRenderer, so empty content shows an empty paragraph instead of the orb empty-state that FileApp/FolderApp use. Where: `src/components/content.rs:371-381,334`, `src/components/file.rs:245-258`, `src/components/folder.rs:212,255-260`. Fix: gate SlateRenderer on `has_rich_content` and show an orb empty state otherwise.
- Delete swallows `delete_node_members` errors (partial): ContentApp and FileApp ignore the member-cleanup result before deleting the node, so a failure can orphan member rows (absent a DB cascade). Where: `src/components/content.rs:291-294`, `src/components/file.rs:193`. Fix: check the result, or only delete the node if member cleanup succeeded.

### Navigation

- Inaccurate "hidden apps" comment: the comment lists `admin` among apps hidden from nav, but admin is actually pushed into the app list and shown. Where: `src/components/layout/breadcrumbs.rs:515,506-514`. Fix: remove `admin` from the comment's hidden list.
- Missing breadcrumb labels for deep-link-only apps: `app_crumb_label()` lacks cases for program, graph, social, map, redirect, cow, perm, so those show raw keys in breadcrumbs. Where: `src/components/layout/breadcrumbs.rs:335-348,346`, `src/components/loader.rs:116-129`. Fix: add cases plus mime keys (including new `redirect`/`cow` keys) in `src/i18n.rs`.
- Screen app has a breadcrumb label but is deliberately not in the app rail: `mime.screen` label exists while `context_apps()` never adds screen, an asymmetry. Where: `src/components/layout/breadcrumbs.rs:343,515-517,450-520`, `src/components/layout/mod.rs:221-230`. Fix: either expose screen in the rail or drop its breadcrumb label (deep link still works).

### Actions/Tool sheet

- Copy-link gating differs by node type: ContentApp shows CopyLinkAction unconditionally, but FolderApp gates it on `(is_auth && count > 0) || is_context_owner`, so an anonymous visitor can copy a document link but not a folder link. Where: `src/components/content.rs:153`, `src/components/folder.rs:115-119`. Fix: make copy-link unconditional in FolderApp (read-only action).
- Export gating differs by node type: document export is unconditional but folder export requires `is_auth && count > 0`. Where: `src/components/content.rs:151`, `src/components/folder.rs:121-122`. Fix: align the two (prefer removing the folder gate).
- Edit affordance missing for folders/groups/events (and poll): ContentApp links to the editor when `can_edit`, but FolderApp and PollApp have no editor link, though the editor accepts wiki/group and wiki/event. Where: `src/components/content.rs:244-253`, `src/components/folder.rs:116-207`, `src/components/editor.rs:106`. Fix: add an editor Link in FolderApp for group/event gated on `is_context_owner && node.mutable && !parent_path.is_empty()`.
- Bluesky share missing for folders/groups/events, wiki/file, and vote/poll: share appears only on ContentApp-rendered nodes. Where: `src/components/content.rs:156-185`, `src/components/folder.rs:116-207`, `src/components/file.rs:143-167`, `src/components/vote/poll.rs:326-546`. Fix: add shareBluesky to FolderApp (and optionally file/poll) following ContentApp's `is_auth && bsky_linked` pattern, or document the intentional limit.

### Permissions

- Speaker-list context-owner lockout (High) is listed under Top inconsistencies. The remaining permission findings (question-vs-comment delete asymmetry, poll close labeling, add-candidate) appear under Top and Voting above.

### Design system

- `.btn-icon` icon size (20px) is set far from the button rule and differs from the default 22px: the size comes from `assets/style.css:5091-5093`, not the `.btn-icon` block at `:1789-1811`, and differs from default `.material-icons` at `:1396-1412`. Fix: co-locate the rule in `.btn-icon` and document (or unify) 20px vs 22px.
- Native `<select>` uses raw px instead of tokens: `padding: 10px`, `border-radius: 8px`, `font-size: 15px` are hardcoded while text fields use typescale/shape tokens. Where: `assets/style.css:1985-2000`; contrast `:1880-1887`. Fix: use `--md-sys-shape-corner-medium`, `--md-sys-typescale-body-large-size`, and spacing tokens.

### i18n

- Duplicate EN `vote.noAmendments`: defined twice (`src/i18n.rs:299` "No amendments" and `:302` "No amendments yet"); the parser keeps the second, so 299 is dead. DA defines it once (`:710`). Fix: remove `:299`, or rename one key if the two states are intentionally distinct.

### States

- Home list shows raw error text instead of ErrorState: group/event load failures render a muted `<p>{e}`, unlike comments/profile/speak/vote which use the ErrorState component. Where: `src/components/home_list.rs:119-120,169-170`; pattern at `src/widgets/feedback.rs:37-46`. Fix: use ErrorState (with separate error logging) in home_list.
- VoteApp loading spinner lacks the card wrapper used by its empty state (partial): the load state is a bare `spinner-overlay`, while the no-poll empty state is wrapped in a card. Where: `src/components/vote/mod.rs:119-123,73-85`. Fix: wrap the loading spinner in the same card, or use a ballot-shaped skeleton.

## Capability matrix

Node-type x ToolSheet/action capability, as exposed in the UI today. "yes" = affordance present, "no" = missing, "gated" = present but conditionally, "n/a" = not applicable. Documents/candidates and the vote content types render via ContentApp; groups/events/folders via FolderApp; polls via PollApp.

| Capability | wiki/document | vote/candidate | vote/policy · change · position | wiki/group · event | wiki/folder | wiki/file | vote/poll |
|---|---|---|---|---|---|---|---|
| Project on screen | yes | yes | yes | **no** | **no** | **no** | **no** |
| Read-only projector view | yes | yes | partial | **no** | **no** | n/a | yes |
| Focus/section control | yes | yes | yes | yes | yes | **no** | **no** |
| Comments-on-screen toggle | yes | yes | yes | **no** | **no** | **no** | **no** |
| CommentSection mounted | yes (no post) | yes (no post) | policy/change: yes · position: yes (no post) | yes (no post) | **no** | yes (no post) | no |
| Delete action | yes | yes | yes | **no** | **no** | yes | **no** |
| Edit action | yes | yes | yes | **no** | **no** | yes (no chips) | **no** |
| Bluesky share | yes | yes | yes | **no** | **no** | **no** | **no** |
| Copy link | yes (ungated) | yes | yes | gated | gated | gated | n/a |
| Export | yes (ungated) | n/a | n/a | gated | gated | n/a | n/a |
| Author chips shown | yes | yes | yes | n/a | n/a | **no (but stored)** | n/a |

The dominant pattern is the FolderApp column (wiki/group, wiki/event, wiki/folder): it is missing project, read-only projector view, comments toggle, delete, edit, and Bluesky, and gates copy-link/export that documents leave open.

## Suggested fix order

Quick wins (localized, low risk):

1. Add the three missing vote i18n keys and remove the duplicate `vote.noAmendments` (`src/i18n.rs`).
2. Fix the outdated "hidden apps" nav comment (`breadcrumbs.rs:515`).
3. Add error handling (match + snackbar) to the silent mutations in `speak.rs`, `sort.rs`, add-question (`position.rs`), and AddChangeButton, and check `delete_node_members` results in content/file delete.
4. Add `:disabled` styles to all button variants (`assets/style.css`).
5. Fix the speaker-list context-owner gate in `speak.rs` (dual ownership check).
6. Add the edit-member save `disabled`/validation and the `invite.sent` feedback message (`member.rs`).
7. Use ErrorState in `home_list.rs`; wrap the VoteApp loading spinner in a card.

Design-system consistency:

8. Migrate raw px spacing to `--md-sys-spacing-*` tokens and align `<select>` and `.btn-icon` icon sizing.

Structural (touches routing and shared components):

9. Give FolderApp the full ContentApp action set for group/event/folder (project, edit, delete, Bluesky, comments-on-screen toggle) and ungate copy-link/export, so the capability matrix's FolderApp column fills in.
10. Add projector-aware branching for wiki/folder/group/event in `MimeLoader` (and consider passing `projector` to PolicyApp/PositionApp) for clean read-only room screens.
11. Resolve the comment/permission model: mount CommentSection only where vote/comment is insertable (or add an "unavailable" state), and reconcile the question-vs-comment delete asymmetry into one consistent rule.
12. Add confirmation dialogs and pending/disabled states to destructive speaker/member operations; add inline add-candidate once photo upload exists.
