# RadikalWiki Dioxus — test & fix plan

Goal: get `web/wiki-dioxus` to behave like the reference React app in
[`web/wiki`](../wiki) against the same NHost/Hasura backend (production is
<https://radikal.wiki>; test with a real account — never commit credentials).

The initial port (scaffolding through all screens) is done; this plan is about
**verifying each area against the real backend and fixing what's broken**, and
(see below) **moving the UI onto the dioxus-components style and component set**.

## North star: Material Design 3, 100%

The active goal is to make the whole UI follow **Material Design 3** — both its
**style** (colour, typography, shape, elevation, state layers, motion) and its
**principles** — validated component-by-component and page-by-page with the
screenshot + contrast tooling (`test-browser.nu --shots`, `test-contrast-audit.js`).
Reference: <https://m3.material.io/>.

**Colour (done — replaceable).** The palette is a genuine M3 scheme generated
from the Radikale Venstre brand seeds by `scripts/gen-theme.ts`
(Google `material-color-utilities`) into `assets/m3-theme.css` as canonical
`--md-sys-color-*` roles (light + dark). **Green `#02944F` leads as the primary,
magenta `#D2307E` is the tertiary accent** ("de to farver bør altid optræde
sammen"). The app's `--md-*` tokens alias the sys roles, so re-skinning the
entire app = change the two seeds and re-run the generator. Green drives the
chrome (bar, buttons, avatars, app-rail, chips); magenta pops on the FAB, the
user avatar and badges/comment avatars.

**Remaining M3 conformance (per component, validate as you go):**
- **Shape** — M3 corner-radius scale (extra-small 4 / small 8 / medium 12 /
  large 16 / full). FAB is a rounded rectangle (done); audit cards, buttons,
  chips, dialogs, text fields.
- **Elevation & surfaces** — use the tonal `surface-container-*` levels for
  elevation instead of ad-hoc shadows; M3 app bars are `surface`-coloured on
  scroll (today the bar is a solid primary — a deliberate brand choice to
  revisit).
- **Typography** — adopt the M3 type scale (display/headline/title/body/label)
  with consistent sizes/weights/line-heights.
- **State layers** — hover/focus/pressed/dragged overlays at M3 opacities
  (8/10/10/16%) on interactive elements.
- **Components** — buttons (filled/tonal/outlined/text/elevated), FAB, chips,
  cards, lists, navigation (rail/bar/drawer), dialogs, snackbars, switches,
  selects, tooltips — each matched to its M3 spec (many are dioxus-primitives).
- **Motion** — M3 easing/duration tokens for transitions.

## Design direction: dioxus-components / dioxus-primitives

Adopt the [dioxus-components](https://github.com/DioxusLabs/dioxus-components)
look and component model as the target UI style. Concretely:

- **Follow that visual style.** The current CSS is a bespoke Material-ish theme
  in `assets/style.css`; migrate it toward the dioxus-components (shadcn-style)
  design — its tokens, spacing, radii, and neutral palette — rather than MUI.
  New UI should be built in that style, and existing screens restyled to match
  as they are touched.
- **Use components from dioxus-components where it makes sense.** It ships
  accessible (WAI-ARIA) primitives via `dioxus-primitives` (added with
  `dx components add`). Prefer them over the hand-rolled equivalents, especially
  the interactive, a11y-sensitive bits we currently open-code:
  - the user-menu **dropdown/popover** (no outside-click / keyboard handling
    today), the delete/confirm and submit **dialogs** (currently two-click
    buttons), **tooltips**, **tabs** for the app views, and the form controls
    (**select**, **checkbox**/**radio** in the poll ballot, **switch** for the
    theme toggle).
- **Isolate our own reusable components.** Factor the app-agnostic pieces with
  no wiki/GraphQL knowledge into a shared module (later possibly a sibling
  crate). Started: `components::ui` holds the generated dioxus-primitives, and
  `components::widgets` holds our own (`Spinner`, `Chip`, poll `Bar`) — screen
  components compose those. Still to extract: `Card`/`ListItem`/`Avatar`
  wrappers, `Snackbar`.
- **Upstream what generalises.** Anything we build that is genuinely generic and
  higher-quality than what dioxus-components has (or missing there) is a
  candidate to contribute back upstream.

Do this **incrementally**, not as a big-bang rewrite: swap one hand-rolled
component for a primitive (or move one into `components/ui`) at a time, keep the
browser tests green, and prefer the highest-duplication / weakest-a11y pieces
first. Each migration should keep `just test` + `just test-browser` passing.

**Migrated so far** (the weak-a11y interactive pieces): user-menu
**dropdown** (custom, with backdrop + keyboard-closable), poll ballot
(`radio_group` + `checkbox`), delete confirm (`AlertDialog`), theme toggle
(`switch`, as a labelled row that flips `html[data-theme]`), and the
**snackbar** now announces (`role=status` + `aria-live=polite`). Add-content
deliberately stays on a native `<select>` (accessible) until the shadcn
`Select` trigger renders the option *label* rather than the raw value.
**Deferred** as low-value churn (restyle as-touched, not a big-bang): wrapping
the 100+ styled `.card` / `.btn` / `.avatar` divs in `Card`/`Button`/`Avatar`.
A full `toast`-provider swap for the snackbar is possible (the primitive
exists) but would rewire ~14 `show_snackbar` call sites for equivalent UX.

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
- `[x]` **admin / perm / map / screen** (were deferred; React shipped admin &
  perm as empty stubs). Now implemented and verified live via `?app=`:
  - **screen**: the context's active node (MimeLoader) beside the speaker list.
  - **admin**: a live results grid — every poll in the context with per-option
    tallies and totals.
  - **perm**: the context's permission rows (mime · role · insert/select/delete).
  - **map**: a full-height OpenStreetMap view (OSM embed, centred on Denmark).
  All four are reachable from the app rail.

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
  token) → subscribe → `next` (answering keepalive pings), surfaced as a signal;
  a `use_live` helper ties it to a component's refresh counter. Used by the
  folder children, poll results, home context list, home invitations and speaker
  list. Verified live end to end: a node inserted by a **separate client**
  appears in an open folder within ~2s with no reload.
- `[x]` Speaker countdown timer (from the list's `time`/`updatedAt`).
- `[x]` Poll result tallies (bar + count/percent per option).
- `[x]` Editor formatting: a Bold/Italic/Code toolbar that wraps the selection,
  plus inline markdown (`**bold**`, `*italic*`, `` `code` ``) mapping to Slate
  marks the renderer displays. Verified live: Bold wraps and renders bold.

The port now covers every RadikalWiki flow end to end, all verified against the
live backend, with real-time updates and full create/read/update/delete.

## Known parity gaps (small)

- **Breadcrumb collapse.** React collapses breadcrumb segments (MUI `Collapse`,
  expand/scroll-into-view) on deep paths; our trail renders every segment. Add a
  collapse (middle segments → `…`, expandable) for long paths.
- Optional content inline image edit; `Card`/`ListItem`/`Avatar`/`Snackbar`
  still hand-rolled (see the migration section above).

## Backlog — from RadikalWiki GitHub issues

Triaged from <https://github.com/RadikalWiki/radikalwiki/issues>. Only issues
that apply to the Dioxus frontend are listed; legacy React/TS-only ones are
excluded (see "Ignored"). `#N` = issue number.

### Apps to port / add

- `#154` "Dioxus" — the umbrella tracking issue for this whole port.
- `[x]` `#68` Graph app — SVG node-link view of the context + children (rail +
  `?app=graph`), child boxes link into nodes.
- `[x]` `#60` Program app — numbered agenda timeline of the context's children
  (rail + `?app=program`).
- `[x]` `#57` Redirect app — node forwards to a `data.url` target; owner can set
  it (`?app=redirect`).
- ~~`#53` WebDAV app~~ — skipped (owner request).
- `[x]` `#78` Profile app — signed-in user + their groups/events (rail +
  `?app=profile`).
- `[x]` `#137` Social wall app — **Bluesky only** via the public AppView
  `searchPosts` (rail + `?app=social`); Mastodon/PixelFed ignored.
- `[x]` `#82` Secret cow app — cowsay easter egg at `?app=cow` (not in the rail).
- `[x]` `#149` Missing-parent app — lists nodes with a null `parentId` (orphans),
  excluding the legitimate root (rail + `?app=parent`).
- ~~`#18` Pixel app~~ — skipped (owner request).

All the above build + render live (browser smoke test asserts each mounts).

### Speaker list

- `[x]` `#6` Allow hiding the speaker list — owner admin panel opens/closes
  (locks) the list; when closed the join panel is hidden.
- `[x]` `#7` Make the speaker list sortable — owner move-to-top / move-to-bottom
  via an `index` override; default keeps procedural-priority + arrival order.
- `[x]` `#13` Support multiple speaker-list instances per context — every
  `speak/list` child renders as its own card.
- `[x]` `#14` Simpler design — current speaker + next highlighted, MM:SS
  countdown pill, speak-type icon per row.
- Also fixed the queue ordering to match React (`data` desc = procedural motions
  jump the queue, then `createdAt` asc). Pure ordering/countdown unit-tested;
  build renders live. Admin/reorder flows not yet driven end-to-end in a test.

### Editor

- `[x]` `#92` Line-break support (shift-enter) — handled natively by the
  contenteditable surface (Shift+Enter inserts a `<br>`).
- `[x]` `#94` Sticky formatting toolbar on long documents — `.editor-toolbar`
  is `position: sticky; top: 0` (`assets/style.css`).
- `[x]` `#97` Auto-link URLs and emails — on save, `dom_to_slate` wraps bare
  `http(s)://` / `www.` / email words in a `link` mark (`richtext::link_segments`,
  conservative so `main.rs`-style prose is left alone). Unit-tested.

### Voting / policy

- `[x]` `#27` Randomise the order of voting options — ballot order is shuffled
  once per mount (Blank kept last); unit-tested.
- `[x]` `#112` Show all sub-changes as a tree in the policy app — amendment /
  poll / comment rows link into their nodes, so a `vote/change` drills into its
  own PolicyApp and the whole amendment tree is browsable.
- `#138` Replace "questions" with a comment model.

### Content / nodes

- `#25` Content metadata attributes (e.g. a "keep longer" flag for programs).
- `[x]` `#32` Comment system — nested Bluesky-style threads (`components/comments.rs`),
  shown under documents / policies; backend permissions in the Hasura perms table.
- `[x]` `#34` "Newest contents" — a recent-content card on the home page
  (`RecentContents`), each item resolving to its full path on click.
- `[x]` `#44` Don't create the node until the first save — satisfied by design:
  the add-content form only inserts on the explicit Add action (no pre-created
  draft node like the React flow had).
- `#69` Node revision/history table.
- `[blocked]` `#108` Remove hardcoded mime lists (drive icons/apps from the mime
  data) — not feasible frontend-only. The `mimes` table's `icon` is only a
  letter/number/questionmark avatar-*mode* hint (verified live: every mime is
  `questionmark` except vote policy=`letter`, candidate/change=`number`), not a
  Material icon; and `MimeLoader` maps mime → a Rust component (compile-time
  code, not data). The one datum that IS present, `hidden`, is already used
  (`loader.rs` filters on `mime.hidden`). Needs the backend to carry real
  icon/app data first.
- `[x]` `#111` Limit node name length — `maxlength` on the name inputs.
- `[x]` `#114` Zoom/maximise images — click-to-zoom lightbox (`ZoomableImage`) in
  the file, content-block and inline-content image views.
- `#115` Live collaborative editing.
- `[x]` `#117` Table of contents — heading blocks become anchored links in a TOC
  above documents with two or more headings; unit-tested.
- `[x]` `#119` MS Office viewer dark mode — invert + hue-rotate filter on the
  viewer iframe under `[data-theme="dark"]`.
- `[x]` `#125` Folder grid view mode — list/grid toggle in the folder header.
- `[~]` `#128` Audio / MIDI file support — audio (and video) already preview via
  native `<audio controls>` / `<video controls>` in the file viewer (`file.rs`).
  MIDI is the only gap: browsers have no native MIDI synth, so it needs an
  external JS synthesizer + soundfont (a CDN dependency) to actually play.
- `[x]` `#143` Show the child count in every content overview — count badge in
  the folder header.

### Members / contexts / permissions

- `#41` Export event participants.
- `[x]` `#51` Allow users to be hidden in groups — owner can hide/unhide a
  member (`set_member_hidden`); hidden members are dropped for non-owners.
- `[x]` `#132` Event viewer inside groups — a folder/group's child events are
  listed in their own "Events" section above the rest of its content.
- `[x]` `#133` Integrate the invite list into the home list — the home screen
  already shows pending group/event invites inline with accept / decline.
- `#134` "Open" contexts anyone can join.
- `#147` New permission system (informs the perm app).

### Design / UX / platform

- `[x]` `#37` Atkinson Hyperlegible font for accessibility — the app font family.
- `[x]` `#73` Pull-to-refresh — drag down (touch) or over-scroll up (wheel) at the
  top reloads the current view with a spinner (`components/pull_refresh.rs`).
- `[skip]` `#118` Move the toolbar to the right-hand bar — skipped by request
  (the current sticky horizontal toolbar stays).
- `[x]` `#122` Refresh data on window focus — `use_live` re-fetches when the
  window regains focus (via `use_focus_refresh`), recovering a dropped socket.
- `[skip]` `#158` New bottom-bar design (list menu · app select · tools) —
  skipped by request (the current bottom bar / desktop app-rail stays).
- `[~]` `#33` PWA / offline mode — installable: web manifest + icon + theme-color
  (`assets/manifest.json`, `assets/icon.svg`, wired in `main.rs`). A service
  worker (`assets/sw.js`) is registered for offline, but full offline needs it
  served from the site ROOT (`/sw.js` or `Service-Worker-Allowed: /`); at
  `/assets/sw.js` its scope is limited — a deploy concern (`src/pwa.rs`).
- `[~]` `#139` Native notifications — "your turn to speak" fires when a speaker
  reaches the top of the queue (permission requested on Join). Poll-open
  notifications still to do (needs new-poll detection).
- `#145` Error/stacktrace reporting API.

### Ignored (legacy React/TS only)

- `#85` React strict mode (react-beautiful-dnd / devexpress).
- `#45` Port build to deno/bun.
- `#146` Use the Plate editor (React/Slate-only lib).
- `#95`, `#96` Slate-specific editor bugs (our editor is a textarea, N/A).

### Uncertain — need your input (questions prepared)

- `#123` "MimeAvatar path on screen" — unclear which behaviour.
- `#155` Find an nhost alternative — backend/infra, out of the frontend port?
- `#135`, `#136` Native DB primitives / get-index DB function — backend work.
- `#153` Register campaign activity — large new feature; scope/priority?

## Cross-cutting checks

- **GraphQL correctness:** every filtered query must omit unset fields (Hasura
  rejects `null` comparison expressions). This bit the home list; grep for
  `NodesBoolExp`/comparison structs when adding queries and prefer
  `..Default::default()` + `skip_serializing_if`.
- **Permissions:** queries run with the user token; compare row visibility with
  `web/wiki` for the same account.
- **Field naming:** cynic maps snake_case Rust fields to camelCase GraphQL
  (`mime_id` → `mimeId`); the schema is camelCase.
