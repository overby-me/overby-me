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

## Known issues (fix first)

1. **Flaky wasm panic on authenticated load (Servo).** Loading a page that
   already has a stored session sometimes traps with `unreachable executed`
   during instantiation; a fresh logged-out→login flow is reliable. Next steps:
   - Add `console_error_panic_hook` in `main.rs` so traps print a real message
     instead of a bare `unreachable` — the single highest-value change for
     debugging everything else.
   - Suspect a Dioxus signal borrow panic (`already borrowed`) on the first
     authenticated render (drawer `HomeList` + `use_resource` reading
     `SESSION`). Audit for `.read()`/`.write()` overlap across an `.await`.
   - Confirm whether it also reproduces in Chrome/Firefox (likely Servo-only).

2. **Navigation into a group/event.** `resolve_path` now walks from the root
   node (key `root`), which matches how `path_from_id` builds URLs. Verify a
   click on a drawer item actually opens the context (blocked from browser
   verification by issue 1). The queries are confirmed correct against the API.

3. **Release build won't load in Servo** (`Module fetching failed`). Only the
   debug build runs there. Fine for dev/testing; investigate before relying on
   Servo for production-build checks (loads fine in Chrome/Firefox).

## Parity areas to verify against web/wiki

Each needs a real-backend pass; `[?]` = not yet verified end to end.

- `[x]` Auth: sign in / register / reset — sign-in verified live.
- `[x]` Home list: groups + events (owned or accepted member), events by year.
- `[?]` **Drawer node tree** (`MenuList` in wiki): wiki swaps the home list for
  a lazy, expandable child tree once inside a context. The Dioxus drawer only
  renders the home list — port `MenuList` for in-context navigation.
- `[?]` Folder view: child list, icons, "not submitted" state, ordering.
- `[?]` Content: Slate.js JSON → read-only render; author line; attachments.
- `[?]` File: image / video / audio / PDF / download.
- `[?]` Editor: contenteditable save/publish (wiki uses Slate).
- `[?]` Voting: policy / change / poll / position / candidate.
- `[?]` Speak: speaker queue join/remove.
- `[?]` Members + invites: list, invite by email, accept/decline.
- `[?]` Sort: drag-and-drop reordering + save.
- `[?]` Search: live results (uses the same `_ilike` filter now un-broken).
- `[?]` Breadcrumbs, `?app=` routing, snackbars, i18n (Da/En), theme.
- `[ ]` Not ported yet from wiki: admin, perm, map, screen — defer.

## Cross-cutting checks

- **GraphQL correctness:** every filtered query must omit unset fields (Hasura
  rejects `null` comparison expressions). This bit the home list; grep for
  `NodesBoolExp`/comparison structs when adding queries and prefer
  `..Default::default()` + `skip_serializing_if`.
- **Permissions:** queries run with the user token; compare row visibility with
  `web/wiki` for the same account.
- **Field naming:** cynic maps snake_case Rust fields to camelCase GraphQL
  (`mime_id` → `mimeId`); the schema is camelCase.
