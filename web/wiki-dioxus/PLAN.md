# RadikalWiki Dioxus — test & fix plan

Goal: get `web/wiki-dioxus` to behave like the reference React app in
[`web/wiki`](../wiki) against the same NHost/Hasura backend (production is
<https://radikal.wiki>; test with a real account — never commit credentials).

The initial port (scaffolding through all screens) is done; this plan is about
**verifying each area against the real backend and fixing what's broken**.

## How to test

- **Unit** — `just test` covers pure logic (GraphQL filter serialization, path
  helpers). Add a test whenever a bug turns out to be wire-format/logic shaped.
- **Browser** — `just test-browser` drives the real app in headless Servo over
  WebDriver (see [`README.md`](./README.md)). Unauthenticated smoke tests run by
  default; `WIKI_EMAIL=… WIKI_PASSWORD=… just test-browser` adds authenticated
  checks against the live backend.
- **Manual** — `just dev` + Servo/Chrome. Watch Servo stderr for
  `RadikalWiki starting…`, `log::*` output, and wasm traps.

Workflow for each area below: reproduce in `web/wiki`, reproduce in
`wiki-dioxus`, diff the GraphQL the two send, fix, then lock it in with a unit
test and/or a `test-browser.nu` assertion.

## Known issues

1. ~~**Flaky wasm panic on authenticated load (Servo).**~~ **FIXED.** The trap
   was a real panic, not a nondeterministic one: `main()` wrote to the `SESSION`
   / `LANG` `GlobalSignal`s *before* `dioxus::launch`, which is only legal
   inside the runtime, so it fired exactly when localStorage already held a
   session. Init now runs in an `App` `use_hook`. `console_error_panic_hook`
   surfaces real messages. (commit: global-signal init.)

2. ~~**Navigation into a group/event.**~~ **FIXED + verified.** Two bugs: the
   catch-all `PathPage` did not re-resolve on client-side navigation between two
   nodes (`use_resource` only re-runs on reactive reads, not prop changes) — now
   keyed on the path so it remounts; and the drawer tree click path is verified
   live.

3. **Release build won't load in Servo** (`Module fetching failed`). Only the
   debug build runs there. Fine for dev/testing; loads fine in Chrome/Firefox.
   Not yet investigated.

## Parity areas vs web/wiki

`[x]` = ported and verified against the live backend · `[~]` = partial ·
`[ ]` = open.

- `[x]` Auth: sign in / register / reset — sign-in verified live.
- `[x]` Home list: groups + events (owned or accepted member), events by year.
- `[x]` **Drawer node tree** (`MenuList`): lazy expandable child tree in-context,
  ancestors auto-expand, active row highlighted. Verified live.
- `[x]` Folder view: child list, icons, "not submitted", index+time ordering,
  hidden-mime filtering. Verified live.
- `[x]` Content: Slate JSON → read-only render + author chips (members).
  Verified live. `[ ]` optional content image not yet shown.
- `[x]` File: image verified live (loads with token); video/audio/PDF/download
  share the same URL path (code present, image path exercised).
- `[x]` Editor: contenteditable save/publish via a typed update mutation —
  persistence verified live. `[ ]` still a plain-text→paragraph serialization,
  not full Slate.
- `[x]` Voting: poll ballot (radio/checkbox, Blank-alone, min/max), cast a
  vote, "you have voted" state. Verified live end to end on a test poll.
- `[x]` Speak: reads the real `speakerlist` child's queue; join/remove wired
  (insert path verified via the shared null-field fix).
- `[x]` Members: real member list + author chips, and invite by email
  (insertMember). Verified live.
- `[x]` Invitations: home list of pending group/event invites with
  accept (updateMember) / decline (deleteMember). Verified live end to end.
- `[x]` Sort: drag-and-drop reorder + save (typed index mutation). Renders +
  save path verified.
- `[x]` Search: live `_ilike` results — verified live.
- `[x]` `?app=` routing (modelled in the route), i18n (Da/En incl. the ported
  vote/speak/poll/sort/invite/member sections), theme, snackbars, breadcrumbs
  (resolved node names, verified live).
- `[ ]` Not ported from wiki: admin, perm, map, screen — deferred.

## Content lifecycle (CRUD)

- `[x]` Create: add a document or folder from the folder view (inline form).
- `[x]` Read: all node types render (see the parity list above).
- `[x]` Update: edit content as text + publish/submit; drag-sort reordering.
- `[x]` Delete: remove a document (two-click confirm) then go to the parent.
- `[x]` `?app=vote` resolves the context's `active` relation to the open poll.
  All verified live against the backend.

## Real-time & rich features

- `[x]` **GraphQL subscriptions** over the Hasura WebSocket
  (`graphql-transport-ws`): `src/subscription.rs` does connection_init (bearer
  token) → subscribe → `next`, surfaced as a signal. The speaker list uses it
  for live updates (a pushed change re-runs the query). Protocol verified live
  end to end (connection_ack + data). Other views can adopt the same hook.
- `[x]` Speaker countdown timer (from the list's `time`/`updatedAt`).
- `[x]` Poll result tallies (bar + count/percent per option).
- `[x]` Editor inline formatting via markdown (`**bold**`, `*italic*`,
  `` `code` ``) mapping to Slate marks the renderer displays.

## Remaining nice-to-haves

- Adopt the subscription hook for the other live views (poll status, invite
  count) — currently those refresh after their own mutations.
- A WYSIWYG toolbar (the editor is markdown-driven, not click-to-format).

## Cross-cutting checks

- **GraphQL correctness:** every filtered query must omit unset fields (Hasura
  rejects `null` comparison expressions). This bit the home list; grep for
  `NodesBoolExp`/comparison structs when adding queries and prefer
  `..Default::default()` + `skip_serializing_if`.
- **Permissions:** queries run with the user token; compare row visibility with
  `web/wiki` for the same account.
- **Field naming:** cynic maps snake_case Rust fields to camelCase GraphQL
  (`mime_id` → `mimeId`); the schema is camelCase.
